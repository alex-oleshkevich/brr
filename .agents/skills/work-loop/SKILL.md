---
name: work-loop
description: How to carry a change from start to hand-back in brr — group related tasks into batches, build and verify each batch together, lint and test what it touches, live-test it, fix findings, iterate at most three times, then stop for the user or ask for review. Use for every implementation task, bead or epic, before writing the first line and before saying anything is done.
---

# Work loop

## 1. Batch

Group related tasks into batches: tasks that touch the same module, share a contract, or can only be
verified together. A batch is the unit you build, verify and report. Unrelated work goes in a
separate batch, never mixed in.

Before starting, check beads (`bd show`, `bd memories`) for decisions already agreed; record every
new decision in beads the moment it is made, so "we agreed" is always checkable.

## 2. Build the batch

Do every task in the batch before verifying any of it, so the checks see the batch as it will ship.
Make surgical edits to the lines that change; rewrite a whole file only when most of it changes.

## 3. Verify the batch

Every round runs all of these against the whole batch:

1. **Lint and tests of the affected features.** `just fmt`, `just lint`, then the tests covering
   what changed (`just test`, or `just test-one …` while iterating). Zero warnings.
2. **Live test. Always, for every change, before reporting it.** Run the real binary and exercise
   every changed behavior per the `live-testing` skill: real arguments, real terminal, real exit
   codes and output streams, every key, flag, input and backend the change touches. Judge each
   result against the help text and the agreed behavior, never against the code you just wrote.
   Tests alone are not proof; a change reported without a quoted live capture (curl output, CLI output, UI screenshot) is not done.
3. **Record findings**: every failure, warning, wrong output, rough edge, or gap against the
   task's acceptance criteria.

## 4. Fix and iterate

- Findings → fix them all, then run step 3 again on the whole batch.
- **At most three rounds.** A round is one full pass of step 3.

## 5. Hand back

- **Still failing after three rounds → stop.** Report what fails, what was tried each round, and
  your best hypothesis. Wait for the user; do not start a fourth round or work around it.
- **Clean round → ask the user to review the code.** Report the batch's changed files, what each
  round found and fixed, and the evidence from the final round (commands and their output). Map
  every change to its reason with the code beside it, so the user sees what relates to what. Do
  not close beads or move to the next batch until the user has reviewed.
- **When the user asks "why" or "what is this for"**, answer with concrete examples: the actual
  messages, paths, inputs and outputs, before and after.
