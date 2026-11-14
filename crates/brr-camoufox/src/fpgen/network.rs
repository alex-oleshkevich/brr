use std::collections::{HashMap, HashSet};

use brr_core::Error;

use super::model::{Ordered, invalid};
use crate::pyrandom::PyRandom;

// Cuts off values that are way too low or contaminated, as in fpgen
const BEAM_WIDTH: usize = 1000;
const FINGERPRINT_FIELD: &str = "fingerprint";

// The value ids a node may take; the order is the order they were added, which fpgen's
// uniform fallback draws from
#[derive(Clone, Default)]
pub struct Allowed {
    order: Vec<u32>,
    members: HashSet<u32>,
}

impl Allowed {
    pub fn insert(&mut self, id: u32) {
        if self.members.insert(id) {
            self.order.push(id);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    fn single(id: u32) -> Allowed {
        let mut allowed = Allowed::default();
        allowed.insert(id);
        allowed
    }

    fn contains(&self, id: u32) -> bool {
        self.members.contains(&id)
    }
}

// Conditions on nodes, in the order they were given: node index to allowed value ids
pub type Evidence = Vec<(usize, Allowed)>;

enum Cpt {
    Row(Vec<(u32, f64)>),
    Branch(HashMap<u32, Cpt>),
}

pub struct Node {
    pub name: String,
    parents: Vec<usize>,
    pub possible: Vec<u32>,
    cpt: Cpt,
    // The row used when the table has none for the parent values
    uniform: Vec<(u32, f64)>,
}

pub struct Network {
    pub nodes: Vec<Node>,
    by_name: HashMap<String, usize>,
    // Value id strings by interned number; the model keys its values by these
    pub ids: Vec<String>,
    ancestors: Vec<Vec<bool>>,
}

impl Node {
    fn row(&self, parent_values: impl Iterator<Item = u32>) -> &[(u32, f64)] {
        let mut cpt = &self.cpt;
        for value in parent_values {
            let Cpt::Branch(next) = cpt else {
                return &self.uniform;
            };
            let Some(branch) = next.get(&value) else {
                return &self.uniform;
            };
            cpt = branch;
        }
        match cpt {
            Cpt::Row(row) if !row.is_empty() => row,
            _ => &self.uniform,
        }
    }
}

struct Interner {
    ids: Vec<String>,
    index: HashMap<String, u32>,
}

impl Interner {
    fn intern(&mut self, id: &str) -> u32 {
        if let Some(&interned) = self.index.get(id) {
            return interned;
        }
        let interned = self.ids.len() as u32;
        self.ids.push(id.to_owned());
        self.index.insert(id.to_owned(), interned);
        interned
    }
}

pub fn no_match(what: &str) -> Error {
    Error::InvalidArgument {
        field: FINGERPRINT_FIELD.to_owned(),
        message: format!("no fingerprint matches {what}"),
    }
}

impl Network {
    pub fn build(root: &Ordered) -> Result<Network, Error> {
        let Ordered::Array(definitions) = member(root, "nodes")? else {
            return Err(invalid("nodes is not an array".to_owned()));
        };

        let mut by_name = HashMap::with_capacity(definitions.len());
        for (index, definition) in definitions.iter().enumerate() {
            by_name.insert(text(member(definition, "name")?)?.to_lowercase(), index);
        }

        let mut interner = Interner {
            ids: Vec::new(),
            index: HashMap::new(),
        };
        let mut nodes: Vec<Node> = Vec::with_capacity(definitions.len());
        let mut ancestors: Vec<Vec<bool>> = Vec::with_capacity(definitions.len());
        for (index, definition) in definitions.iter().enumerate() {
            let name = text(member(definition, "name")?)?.to_owned();

            let mut parents = Vec::new();
            let mut reach = vec![false; definitions.len()];
            for parent_name in strings(member(definition, "parentNames")?)? {
                let parent = *by_name
                    .get(&parent_name.to_lowercase())
                    .filter(|&&parent| parent < index)
                    .ok_or_else(|| {
                        invalid(format!(
                            "{name} has parent {parent_name} that is not an earlier node"
                        ))
                    })?;
                parents.push(parent);
                reach[parent] = true;
                for (node, &reaches) in ancestors[parent].iter().enumerate() {
                    reach[node] |= reaches;
                }
            }

            let possible: Vec<u32> = strings(member(definition, "possibleValues")?)?
                .into_iter()
                .map(|id| interner.intern(id))
                .collect();
            let uniform = possible
                .iter()
                .map(|&id| (id, 1.0 / possible.len() as f64))
                .collect();
            let cpt = parse_cpt(
                member(definition, "conditionalProbabilities")?,
                parents.len(),
                &mut interner,
            )?;

            ancestors.push(reach);
            nodes.push(Node {
                name,
                parents,
                possible,
                cpt,
                uniform,
            });
        }

        Ok(Network {
            nodes,
            by_name,
            ids: interner.ids,
            ancestors,
        })
    }

    pub fn node_index(&self, name: &str) -> Option<usize> {
        self.by_name.get(&name.to_lowercase()).copied()
    }

    // Rejects conditions that cannot hold together with the other single-valued ones
    pub fn validate(&self, evidence: &Evidence) -> Result<(), Error> {
        if evidence.len() <= 1 {
            return Ok(());
        }

        for (node, allowed) in evidence {
            let fixed: Evidence = evidence
                .iter()
                .filter(|(other, others_allowed)| other != node && others_allowed.order.len() == 1)
                .cloned()
                .collect();
            if fixed.is_empty() {
                continue;
            }

            // fpgen passes the fixed values as strings, which isAllowed tests as substrings
            let mut pinned = vec![None; self.nodes.len()];
            for (fixed_node, fixed_allowed) in &fixed {
                let fixed_id = self.ids[fixed_allowed.order[0] as usize].as_str();
                let mut substrings = Allowed::default();
                for &id in &self.nodes[*fixed_node].possible {
                    if fixed_id.contains(self.ids[id as usize].as_str()) {
                        substrings.insert(id);
                    }
                }
                pinned[*fixed_node] = Some(substrings);
            }

            let distribution = self.trace(*node, &pinned);
            if !distribution.is_empty()
                && allowed
                    .order
                    .iter()
                    .all(|&id| lookup(&distribution, id) <= 0.0)
            {
                let names: Vec<&str> = fixed
                    .iter()
                    .map(|(fixed_node, _)| self.nodes[*fixed_node].name.as_str())
                    .collect();
                return Err(no_match(&format!(
                    "{} given {}",
                    self.nodes[*node].name,
                    names.join(", ")
                )));
            }
        }
        Ok(())
    }

    // One value id per node, in node order
    pub fn sample(&self, evidence: &Evidence, rng: &mut PyRandom) -> Result<Vec<u32>, Error> {
        let mut current = self.dense(evidence);
        let mut sampled = Vec::with_capacity(self.nodes.len());

        for (index, node) in self.nodes.iter().enumerate() {
            let value = match current[index].take() {
                Some(allowed) => {
                    // The node itself stays out of the search
                    let distribution = self.trace(index, &current);
                    let mut filtered: Vec<(u32, f64)> = distribution
                        .into_iter()
                        .filter(|&(id, _)| allowed.contains(id))
                        .collect();
                    let total: f64 = filtered.iter().map(|&(_, p)| p).sum();
                    if filtered.is_empty() || total <= 0.0 {
                        let uniform = 1.0 / allowed.order.len() as f64;
                        filtered = allowed.order.iter().map(|&id| (id, uniform)).collect();
                    } else {
                        for (_, p) in &mut filtered {
                            *p /= total;
                        }
                    }
                    draw(&filtered, rng)
                }
                None => draw(&self.trace(index, &current), rng),
            }
            .ok_or_else(|| no_match(&node.name))?;

            sampled.push(value);
            current[index] = Some(Allowed::single(value));
        }
        Ok(sampled)
    }

    fn dense(&self, evidence: &Evidence) -> Vec<Option<Allowed>> {
        let mut dense = vec![None; self.nodes.len()];
        for (node, allowed) in evidence {
            dense[*node] = Some(allowed.clone());
        }
        dense
    }

    // The conditional distribution of target given the evidence, by beam search over the
    // target's ancestors and the evidence nodes' ancestors; empty when nothing fits
    pub(crate) fn trace(&self, target: usize, evidence: &[Option<Allowed>]) -> Vec<(u32, f64)> {
        let mut relevant = self.ancestors[target].clone();
        relevant[target] = true;
        for (index, _) in evidence.iter().enumerate().filter(|(_, ev)| ev.is_some()) {
            relevant[index] = true;
            for (node, &reaches) in self.ancestors[index].iter().enumerate() {
                relevant[node] |= reaches;
            }
        }

        let ordered: Vec<usize> = (0..self.nodes.len()).filter(|&i| relevant[i]).collect();
        let mut slot = vec![usize::MAX; self.nodes.len()];
        for (position, &node) in ordered.iter().enumerate() {
            slot[node] = position;
        }

        let mut beam: Vec<(Vec<u32>, f64)> = vec![(Vec::new(), 1.0)];
        for &index in &ordered {
            let node = &self.nodes[index];
            let allowed = evidence[index].as_ref();

            let mut next: Vec<(usize, u32, f64)> = Vec::new();
            for (entry, (values, prob)) in beam.iter().enumerate() {
                let row = node.row(node.parents.iter().map(|&parent| values[slot[parent]]));
                for &(value, p) in row {
                    if allowed.is_none_or(|allowed| allowed.contains(value)) && p > 0.0 {
                        next.push((entry, value, prob * p));
                    }
                }
            }
            if next.is_empty() {
                return Vec::new();
            }
            if next.len() > BEAM_WIDTH {
                // Stable, like heapq.nlargest: equal probabilities keep their order
                next.sort_by(|a, b| b.2.total_cmp(&a.2));
                next.truncate(BEAM_WIDTH);
            }

            beam = next
                .into_iter()
                .map(|(entry, value, prob)| {
                    let mut values = Vec::with_capacity(beam[entry].0.len() + 1);
                    values.extend_from_slice(&beam[entry].0);
                    values.push(value);
                    (values, prob)
                })
                .collect();
        }

        let target_slot = slot[target];
        let mut distribution: Vec<(u32, f64)> = Vec::new();
        let mut position: HashMap<u32, usize> = HashMap::new();
        let mut total = 0.0;
        for (values, prob) in &beam {
            let value = values[target_slot];
            match position.get(&value) {
                Some(&at) => distribution[at].1 += prob,
                None => {
                    position.insert(value, distribution.len());
                    distribution.push((value, *prob));
                }
            }
            total += prob;
        }
        if total <= 0.0 {
            return Vec::new();
        }
        for (_, p) in &mut distribution {
            *p /= total;
        }
        distribution
    }
}

fn lookup(distribution: &[(u32, f64)], id: u32) -> f64 {
    distribution
        .iter()
        .find(|&&(value, _)| value == id)
        .map_or(0.0, |&(_, p)| p)
}

// One rng draw, the cumulative scan of fpgen; falls back to the first value
fn draw(distribution: &[(u32, f64)], rng: &mut PyRandom) -> Option<u32> {
    let anchor = rng.random();
    let mut cumulative = 0.0;
    for &(value, p) in distribution {
        cumulative += p;
        if anchor < cumulative {
            return Some(value);
        }
    }
    distribution.first().map(|&(value, _)| value)
}

fn member<'a>(value: &'a Ordered, key: &str) -> Result<&'a Ordered, Error> {
    let Ordered::Object(entries) = value else {
        return Err(invalid(format!("expected an object around {key}")));
    };
    entries
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, member)| member)
        .ok_or_else(|| invalid(format!("missing {key}")))
}

fn text(value: &Ordered) -> Result<&str, Error> {
    match value {
        Ordered::String(text) => Ok(text),
        _ => Err(invalid("expected a string".to_owned())),
    }
}

fn strings(value: &Ordered) -> Result<Vec<&str>, Error> {
    match value {
        Ordered::Array(items) => items.iter().map(text).collect(),
        _ => Err(invalid("expected an array of strings".to_owned())),
    }
}

// depth is the number of parents: one table level per parent, then the row of probabilities
fn parse_cpt(value: &Ordered, depth: usize, interner: &mut Interner) -> Result<Cpt, Error> {
    let Ordered::Object(entries) = value else {
        return Err(invalid("a probability table is not an object".to_owned()));
    };

    if depth == 0 {
        let mut row = Vec::with_capacity(entries.len());
        for (id, probability) in entries {
            let Ordered::Number(number) = probability else {
                return Err(invalid("a probability is not a number".to_owned()));
            };
            row.push((interner.intern(id), number.as_f64().unwrap_or(0.0)));
        }
        return Ok(Cpt::Row(row));
    }

    let mut branches = HashMap::with_capacity(entries.len());
    for (id, next) in entries {
        branches.insert(interner.intern(id), parse_cpt(next, depth - 1, interner)?);
    }
    Ok(Cpt::Branch(branches))
}
