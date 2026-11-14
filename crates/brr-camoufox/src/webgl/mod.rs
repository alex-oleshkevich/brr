use std::collections::BTreeSet;

use brr_core::Error;
use serde_json::Value;

use crate::{
    coherence::{gpu_fits_machine, gpu_fits_os},
    fpgen::{
        model::{Model, Ordered, parse_ordered},
        network::{Allowed, no_match},
    },
    pycompat,
    pyrandom::PyRandom,
};

pub const FPGEN_OS: [(&str, &str); 6] = [
    ("lin", "Linux"),
    ("linux", "Linux"),
    ("mac", "macOS"),
    ("macos", "macOS"),
    ("win", "Windows"),
    ("windows", "Windows"),
];
const BROWSER_NODE: &str = "browser";
const OS_NODE: &str = "os";
const FIREFOX: &str = "Firefox";
const GPU_NODE: &str = "gpu";
const WEBGL_NODE: &str = "webgl";
const WEBGL2_NODE: &str = "webgl2";
const SUPPORTED_EXTENSIONS: &str = "supportedExtensions";
const ALIASED_LINE_WIDTH_RANGE: &str = "33902";
const DEPTH_CLAMP: &str = "EXT_depth_clamp";
// Firefox 158 (bug 1854003) exposes it wherever the GL backend clamps polygon offsets: ANGLE's
// D3D11 backend from feature level 10_0, which WebGL2 needs, and Linux desktop GL drivers whose
// recordings have EXT_depth_clamp. macOS release builds run WebGL on CGL's GL 4.1, which lacks it.
const POLYGON_OFFSET_CLAMP: &str = "EXT_polygon_offset_clamp";
// What Firefox reports under privacy.resistFingerprinting, which a Camoufox identity otherwise
// does not present
const RFP_RENDERER: &str = "Mozilla";
const MODEL_FIELD: &str = "webgl";
const NETBOOK_MAX_PIXELS: i64 = 1024 * 600;
const NVIDIA_PREFIX: &str = "NVIDIA ";

// Draft extensions behind webgl.enable-draft-extensions, or mobile-only: fpgen's corpus carries
// some of them, and a spoofed list that names one is a tell on its own
const NEVER_EXPOSED_EXTENSIONS: [&str; 4] = [
    "WEBGL_multi_draw",
    "WEBGL_clip_cull_distance",
    "EXT_texture_norm16",
    "WEBGL_compressed_texture_etc1",
];
// ANGLE's D3D11 backend has it on every Windows GPU; a Linux or macOS host driver may not, and
// the browser answers from the spoofed list without asking the host
const HOST_DEPENDENT_EXTENSIONS: [&str; 1] = ["OVR_multiview2"];

const SOFTWARE_RENDERERS: [&str; 4] = [
    "llvmpipe",
    "Microsoft Basic Render Driver",
    "SwiftShader",
    "Generic Renderer",
];
// Gecko renderer buckets that are discrete GPUs: no netbook shipped one
const DISCRETE_GPU_BUCKETS: [&str; 4] = [
    "GeForce 8800 GTX",
    "GeForce GTX 480",
    "GeForce GTX 980",
    "Radeon R9 200 Series",
];

struct TraceResult {
    id: u32,
    text: String,
    probability: f64,
}

fn invalid(field: &str, message: String) -> Error {
    Error::InvalidArgument {
        field: field.to_owned(),
        message,
    }
}

fn fpgen_os(target_os: &str) -> Result<&'static str, Error> {
    FPGEN_OS
        .iter()
        .find(|(key, _)| *key == target_os)
        .map(|(_, name)| *name)
        .ok_or_else(|| invalid("target_os", pycompat::str_repr(target_os)))
}

fn node_index(model: &Model, name: &str) -> Result<usize, Error> {
    model.network().node_index(name).ok_or_else(|| {
        invalid(
            MODEL_FIELD,
            format!("{name} is not a node of the fingerprint model"),
        )
    })
}

fn only(id: u32) -> Allowed {
    let mut allowed = Allowed::default();
    allowed.insert(id);
    allowed
}

// The value ids of a node whose JSON text is this string, compared case-insensitively as fpgen
// does for a condition
fn matching_ids(model: &Model, node: usize, wanted: &str) -> Result<Allowed, Error> {
    let network = model.network();
    let wanted = format!("\"{}\"", wanted.to_lowercase());
    let mut allowed = Allowed::default();
    for &id in &network.nodes[node].possible {
        if model.lookup_text(&network.ids[id as usize])?.to_lowercase() == wanted {
            allowed.insert(id);
        }
    }
    Ok(allowed)
}

// fpgen's distribution of `target` for Firefox on `target_os`, in its order: most probable first
fn trace(
    model: &Model,
    target: &str,
    target_os: &str,
    pinned: &[(&str, u32)],
) -> Result<Vec<TraceResult>, Error> {
    let network = model.network();
    let mut evidence: Vec<Option<Allowed>> = vec![None; network.nodes.len()];
    for &(node, id) in pinned {
        evidence[node_index(model, node)?] = Some(only(id));
    }
    for (node, wanted) in [(BROWSER_NODE, FIREFOX), (OS_NODE, fpgen_os(target_os)?)] {
        let index = node_index(model, node)?;
        evidence[index] = Some(matching_ids(model, index, wanted)?);
    }

    let mut distribution = network.trace(node_index(model, target)?, &evidence);
    if distribution.is_empty() {
        return Err(no_match(target));
    }
    // Stable, like list.sort(reverse=True)
    distribution.sort_by(|a, b| b.1.total_cmp(&a.1));

    distribution
        .into_iter()
        .map(|(id, probability)| {
            Ok(TraceResult {
                id,
                text: model.lookup_text(&network.ids[id as usize])?,
                probability,
            })
        })
        .collect()
}

fn choose<'a>(rng: &mut PyRandom, results: &[&'a TraceResult]) -> &'a TraceResult {
    let mut total = 0.0;
    let cumulative: Vec<f64> = results
        .iter()
        .map(|result| {
            total += result.probability;
            total
        })
        .collect();
    results[rng.choices_weighted(&cumulative)]
}

fn parse(text: &str) -> Result<Value, Error> {
    serde_json::from_str(text).map_err(|err| Error::Internal {
        message: format!("decoding a fingerprint value: {err}"),
    })
}

fn gpu_pair(text: &str) -> Result<(String, String), Error> {
    let gpu = parse(text)?;
    let field = |key: &str| gpu[key].as_str().unwrap_or_default().to_owned();
    Ok((field("vendor"), field("renderer")))
}

// Every (vendor, renderer) that fpgen has seen Firefox report on this OS. A GPU outside this set
// has no recorded WebGL parameters behind it, so an identity naming it could only borrow
// another device's.
pub fn firefox_gpus(model: &Model, target_os: &str) -> Result<Vec<(String, String)>, Error> {
    trace(model, GPU_NODE, target_os, &[])?
        .iter()
        .map(|result| gpu_pair(&result.text))
        .collect()
}

fn field<'a>(value: &'a Ordered, key: &str) -> Result<&'a Ordered, Error> {
    let Ordered::Object(members) = value else {
        return Err(invalid(
            MODEL_FIELD,
            format!("expected an object around {key}"),
        ));
    };
    members
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, member)| member)
        .ok_or_else(|| invalid(MODEL_FIELD, format!("missing {key}")))
}

fn extension_names(webgl: &Ordered) -> Result<Vec<String>, Error> {
    let Ordered::Array(items) = field(webgl, SUPPORTED_EXTENSIONS)? else {
        return Err(invalid(
            MODEL_FIELD,
            format!("{SUPPORTED_EXTENSIONS} is not an array"),
        ));
    };
    items
        .iter()
        .map(|item| match item {
            Ordered::String(name) => Ok(name.clone()),
            _ => Err(invalid(
                MODEL_FIELD,
                "an extension is not a string".to_owned(),
            )),
        })
        .collect()
}

// The extensions the shipped Firefox adds on this device's backend
fn backend_extensions(
    target_os: &str,
    webgl: &Ordered,
    has_webgl2: bool,
) -> Result<Vec<&'static str>, Error> {
    let mut extensions = Vec::new();
    if target_os == "win" {
        extensions.push(DEPTH_CLAMP);
    }
    if (target_os == "win" && has_webgl2)
        || (target_os == "lin"
            && extension_names(webgl)?
                .iter()
                .any(|name| name == DEPTH_CLAMP))
    {
        extensions.push(POLYGON_OFFSET_CLAMP);
    }
    Ok(extensions)
}

fn number_text(value: &Ordered) -> Result<String, Error> {
    match value {
        Ordered::Number(number) => Ok(number.to_string()),
        _ => Err(invalid(
            MODEL_FIELD,
            "a precision format key is not a number".to_owned(),
        )),
    }
}

fn context_config(
    prefix: &str,
    webgl: &Ordered,
    target_os: &str,
    backend: &[&str],
) -> Result<[(String, Ordered); 3], Error> {
    let blocked = |extension: &str| {
        NEVER_EXPOSED_EXTENSIONS.contains(&extension)
            || (target_os != "win" && HOST_DEPENDENT_EXTENSIONS.contains(&extension))
    };
    let mut extensions: Vec<String> = extension_names(webgl)?
        .into_iter()
        .filter(|extension| !blocked(extension))
        .collect();
    for &extension in backend {
        if extensions.iter().any(|existing| existing == extension) {
            continue;
        }
        // Firefox lists them in WebGLExtensionID order: alphabetical, ignoring case
        let at = extensions
            .iter()
            .position(|other| other.to_lowercase() > extension.to_lowercase())
            .unwrap_or(extensions.len());
        extensions.insert(at, extension.to_owned());
    }

    let Ordered::Object(params) = field(webgl, "params")? else {
        return Err(invalid(MODEL_FIELD, "params is not an object".to_owned()));
    };
    let parameters = params
        .iter()
        .map(|(pname, param)| Ok((pname.clone(), field(param, "value")?.clone())))
        .collect::<Result<Vec<_>, Error>>()?;

    let Ordered::Array(entries) = field(webgl, "shaderPrecisionFormats")? else {
        return Err(invalid(
            MODEL_FIELD,
            "shaderPrecisionFormats is not an array".to_owned(),
        ));
    };
    let formats = entries
        .iter()
        .map(|entry| {
            let key = format!(
                "{},{}",
                number_text(field(entry, "shaderType")?)?,
                number_text(field(entry, "precisionType")?)?
            );
            Ok((key, field(entry, "shaderPrecisionFormat")?.clone()))
        })
        .collect::<Result<Vec<_>, Error>>()?;

    Ok([
        (
            format!("{prefix}:{SUPPORTED_EXTENSIONS}"),
            Ordered::Array(extensions.into_iter().map(Ordered::String).collect()),
        ),
        (format!("{prefix}:parameters"), Ordered::Object(parameters)),
        (
            format!("{prefix}:shaderPrecisionFormats"),
            Ordered::Object(formats),
        ),
    ])
}

fn is_present(value: &Ordered) -> bool {
    match value {
        Ordered::Array(items) => !items.is_empty(),
        Ordered::Object(members) => !members.is_empty(),
        _ => false,
    }
}

fn parameters_mut<'a>(
    config: &'a mut [(String, Ordered)],
    key: &str,
) -> Result<&'a mut Vec<(String, Ordered)>, Error> {
    config
        .iter_mut()
        .find_map(|(name, value)| match value {
            Ordered::Object(members) if name == key => Some(members),
            _ => None,
        })
        .ok_or_else(|| invalid(MODEL_FIELD, format!("missing {key}")))
}

// fpgen's `webgl` and `webgl2` values as Camoufox config keys, in the order the launchers write
// them. `webgl2` is `[]` for a device without WebGL2.
pub fn to_config(webgl: &Ordered, webgl2: &Ordered, target_os: &str) -> Result<Ordered, Error> {
    let has_webgl2 = is_present(webgl2);
    let backend = backend_extensions(target_os, webgl, has_webgl2)?;

    let mut config = vec![
        ("webGl:vendor".to_owned(), field(webgl, "vendor")?.clone()),
        (
            "webGl:renderer".to_owned(),
            field(webgl, "renderer")?.clone(),
        ),
    ];
    config.extend(context_config("webGl", webgl, target_os, &backend)?);
    config.push(("webGl2Enabled".to_owned(), Ordered::Bool(has_webgl2)));

    if has_webgl2 {
        config.extend(context_config("webGl2", webgl2, target_os, &backend)?);

        // Firefox 156 creates WebGL1 on the same GL profile as WebGL2, so both report one line
        // width range. 146 forced WebGL1 onto a compatibility profile, and fpgen recorded that
        // profile's driver range.
        let range = parameters_mut(&mut config, "webGl2:parameters")?
            .iter()
            .find(|(pname, _)| pname == ALIASED_LINE_WIDTH_RANGE)
            .map(|(_, value)| value.clone())
            .ok_or_else(|| invalid(MODEL_FIELD, format!("missing {ALIASED_LINE_WIDTH_RANGE}")))?;
        let webgl1 = parameters_mut(&mut config, "webGl:parameters")?;
        match webgl1
            .iter_mut()
            .find(|(pname, _)| pname == ALIASED_LINE_WIDTH_RANGE)
        {
            Some(entry) => entry.1 = range,
            None => webgl1.push((ALIASED_LINE_WIDTH_RANGE.to_owned(), range)),
        }
    }
    Ok(Ordered::Object(config))
}

// One device supplies every WebGL1 and WebGL2 value, so the two never disagree; the draws run
// in a fixed order so a seed gives the same device
fn webgl_config(
    model: &Model,
    target_os: &str,
    gpu_id: u32,
    rng: &mut PyRandom,
) -> Result<Ordered, Error> {
    let candidates = trace(model, WEBGL_NODE, target_os, &[(GPU_NODE, gpu_id)])?;
    let webgl = choose(rng, &candidates.iter().collect::<Vec<_>>());

    let candidates = trace(
        model,
        WEBGL2_NODE,
        target_os,
        &[(GPU_NODE, gpu_id), (WEBGL_NODE, webgl.id)],
    )?;
    let webgl2 = choose(rng, &candidates.iter().collect::<Vec<_>>());

    to_config(
        &parse_ordered(&webgl.text).map_err(|err| invalid(MODEL_FIELD, err.to_string()))?,
        &parse_ordered(&webgl2.text).map_err(|err| invalid(MODEL_FIELD, err.to_string()))?,
        target_os,
    )
}

fn rng_for(seed: Option<u128>) -> PyRandom {
    seed.map_or_else(PyRandom::from_os, PyRandom::seed_wide)
}

// The WebGL config of a device with this GPU, as Firefox on `target_os` reports it. A GPU fpgen
// has never seen Firefox report on that OS has no recorded parameters, and another device's
// would contradict it.
pub fn for_gpu(
    model: &Model,
    target_os: &str,
    vendor: &str,
    renderer: &str,
    seed: Option<u128>,
) -> Result<Ordered, Error> {
    let gpus = trace(model, GPU_NODE, target_os, &[])?;
    for gpu in &gpus {
        if gpu_pair(&gpu.text)? == (vendor.to_owned(), renderer.to_owned()) {
            return webgl_config(model, target_os, gpu.id, &mut rng_for(seed));
        }
    }

    let pairs: BTreeSet<(String, String)> = gpus
        .iter()
        .map(|gpu| gpu_pair(&gpu.text))
        .collect::<Result<_, _>>()?;
    let pairs: Vec<String> = pairs
        .iter()
        .map(|(vendor, renderer)| {
            format!(
                "({}, {})",
                pycompat::str_repr(vendor),
                pycompat::str_repr(renderer)
            )
        })
        .collect();
    Err(invalid(
        MODEL_FIELD,
        format!(
            "No recorded WebGL data for vendor {} and renderer {} from Firefox on {}. Possible pairs: [{}]",
            pycompat::str_repr(vendor),
            pycompat::str_repr(renderer),
            fpgen_os(target_os)?,
            pairs.join(", ")
        ),
    ))
}

fn optional(value: Option<i64>) -> Value {
    value.map_or(Value::Null, Value::from)
}

// Draw a GPU for a synthetic identity, weighted as fpgen records Firefox on `target_os`, and its
// WebGL config. Only GPUs the rest of the identity can stand beside are drawn: never a software
// rasterizer, a GPU the OS cannot report, the resistFingerprinting mask, a discrete GPU behind a
// netbook panel, or an Intel Mac GPU beside cores or a panel no Intel Mac has. The screen is left
// alone: it has already been reconciled with the real display and the window.
pub fn sample_for_screen(
    model: &Model,
    target_os: &str,
    width: Option<i64>,
    height: Option<i64>,
    seed: Option<u128>,
    cores: Option<i64>,
) -> Result<Ordered, Error> {
    let gpus = trace(model, GPU_NODE, target_os, &[])?;
    let (cores_value, width_value, height_value) =
        (optional(cores), optional(width), optional(height));

    let mut candidates = Vec::new();
    for gpu in &gpus {
        let (_, renderer) = gpu_pair(&gpu.text)?;
        if !is_software_renderer(Some(&renderer))
            && renderer != RFP_RENDERER
            && gpu_fits_os(Some(&renderer), target_os)
            && gpu_fits_machine(
                Some(&renderer),
                target_os,
                &cores_value,
                &width_value,
                &height_value,
            )
            && gpu_screen_is_plausible(Some(&renderer), width, height)
        {
            candidates.push(gpu);
        }
    }
    if candidates.is_empty() {
        let size = |value: &Value| pycompat::str(value);
        return Err(invalid(
            MODEL_FIELD,
            format!(
                "No recorded {target_os} GPU fits a {}x{} screen",
                size(&width_value),
                size(&height_value)
            ),
        ));
    }

    let mut rng = rng_for(seed);
    let gpu = choose(&mut rng, &candidates);
    webgl_config(model, target_os, gpu.id, &mut rng)
}

pub fn is_software_renderer(renderer: Option<&str>) -> bool {
    renderer.is_some_and(|renderer| {
        SOFTWARE_RENDERERS
            .iter()
            .any(|name| renderer.contains(name))
    })
}

// ANGLE (vendor, name Direct3D...): the name
fn angle_d3d(renderer: &str) -> Option<&str> {
    let inner = renderer.strip_prefix("ANGLE (")?.strip_suffix(')')?;
    let (_, rest) = inner.split_once(',')?;
    let rest = rest.strip_prefix(' ')?;
    Some(&rest[..rest.find(" Direct3D")?])
}

fn angle_vulkan(renderer: &str) -> Option<&str> {
    renderer
        .strip_prefix("ANGLE (")?
        .strip_suffix(") on Vulkan")
}

fn pcie_sse2(renderer: &str) -> Option<&str> {
    renderer
        .strip_suffix("/PCIe/SSE2")
        .or_else(|| renderer.strip_suffix("/PCI/SSE2"))
}

// Reduce a reported renderer to Gecko's sanitized device bucket
pub fn renderer_bucket(renderer: &str) -> &str {
    let core = renderer.strip_suffix(", or similar").unwrap_or(renderer);
    let core = angle_d3d(core)
        .or_else(|| angle_vulkan(core))
        .unwrap_or(core);
    let core = pcie_sse2(core).unwrap_or(core);
    core.strip_prefix(NVIDIA_PREFIX).unwrap_or(core)
}

// Whether `renderer` is a GPU that plausibly drives a `width` x `height` screen; no size, or 0,
// constrains nothing
pub fn gpu_screen_is_plausible(
    renderer: Option<&str>,
    width: Option<i64>,
    height: Option<i64>,
) -> bool {
    let (Some(renderer), Some(width), Some(height)) = (renderer, width, height) else {
        return true;
    };
    if renderer.is_empty()
        || width == 0
        || height == 0
        || is_software_renderer(Some(renderer))
        || !DISCRETE_GPU_BUCKETS.contains(&renderer_bucket(renderer))
    {
        return true;
    }
    width * height > NETBOOK_MAX_PIXELS
}

#[cfg(test)]
mod tests;
