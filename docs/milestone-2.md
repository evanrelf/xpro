# Milestone 2 plan

The working plan for [milestone 2, "A graph, headless first"](roadmap.md). It
records the decisions already made and the steps that remain, so work can pick
up without prior context. Read [design.md](design.md) first. Update this file as
steps land, and fold lasting decisions into the design doc or glossary as
they're settled.

**Done when** you can edit a photo, save it, and rerun the same graph on a
different photo from the command line.

## Where things start

Milestone 1 left a fixed pipeline, not a graph. The core crate has these pieces:

- **Decoding:** a raw file becomes a Bayer mosaic plus metadata, namely the
  as-shot white balance, the camera color matrix and the orientation.
- **Development:** white balance on the mosaic (clipping to keep blown
  highlights neutral), bilinear demosaicing, camera RGB to linear Rec.2020, a
  baseline exposure, and orientation.
- **Viewing:** a sigmoid tone curve and conversion to 8-bit sRGB. The curve's
  default white point is tied to where the sensor clips after the baseline
  exposure, and a test enforces this.
- **Output:** writing a PNG, refusing to overwrite.

The CLI's `render` command and the app both call this pipeline directly. The
app also keeps the photo at several halved sizes so it can draw sharply at any
zoom.

## Decisions

- **Fine-grained nodes, no monolithic "develop" node.** Each step of raw
  development is its own node, with a little configuration where it makes sense.
  Friendlier, higher-level editing comes later from composites built out of
  these primitives.
- **Built-in nodes are an enum of plain structs**, one variant per node type,
  each struct holding its typed parameters. This keeps the file format simple
  through serialization and keeps matching exhaustive. Third-party nodes, if they
  ever come, can be added as a dynamic variant without changing the rest.
- **A node's inputs live inside the node**, as fields next to its parameters,
  rather than in a separate list of connections.
- **Node IDs are generated, readable and unordered:** the node type in snake
  case plus a short random suffix, like `exposure_k3f9`. Nobody types them by
  hand. The random suffix avoids encoding the order nodes were added. A node can
  also have an optional label, a human-readable name that doesn't have to be
  unique.
- **Sidecars are named after the photo's full filename**, including its
  extension, so a raw file and a JPEG with the same name get separate edits.
  Xpro may use more than one file extension for different kinds of file, for
  example a sidecar (a graph bound to a sibling photo) versus a standalone
  graph. Don't force everything into one extension. The names are decided in
  step 0.
- **The command line is sidecar-aware.** `render PHOTO` uses the photo's
  sidecar if it has one, otherwise a default graph. A flag applies some other
  graph instead, for example another photo's sidecar.
- **Headless first.** The core, file format, evaluator and CLI come before the
  GUI, so the GUI lands on a settled core.

## Steps

Each step is one or a few commits, tested headlessly before moving on.

### 0. Glossary

Write `docs/glossary.md` with the canonical terms for Xpro's concepts, then use
only those terms in code, the CLI and docs. Candidates include photo, raw file,
graph, node, node type, input, output, parameter, connection, label, edit,
sidecar, value, scene-referred image, display-referred image, mosaic, cast node,
evaluate, execute, action, viewer, region, preview scale, composite and preset.
Many are already used in the design doc. This is also where the file kinds and
their extensions get named. Draft it, then have the user review it before
building on it.

### 1. Values and nodes, without a graph

- **Value types:** mosaic, scene-referred image and display-referred image.
  A scene-referred image carries its color space, which starts as the camera's
  own RGB and becomes Rec.2020 after conversion. Values also carry the metadata
  later nodes need, such as the as-shot white balance, the camera color matrix
  and the orientation.
- **Node structs,** each with typed parameters and an evaluate function: raw
  input, white balance, demosaic, camera to working space, orient, exposure,
  view transform and PNG output. The baseline exposure becomes the default for
  the exposure node in the default graph, no longer a hidden step.
- Chain the nodes in code to replace the fixed pipeline. Rendered output must
  stay the same, which makes the refactor easy to verify.

### 2. The graph and its file format

- **The graph:** a map from node IDs to nodes, each with an optional label,
  plus a file format version. Serialize it to RON.
- **Validation:** inputs that refer to missing nodes, cycles, and type
  mismatches, such as a display-referred image wired into exposure.
- **Tests:** property tests that graphs round-trip through the file format, and
  that validation catches each kind of error.

### 3. Evaluate and execute

- An evaluator that computes a node's value by evaluating its inputs first, with
  no caching yet.
- Output nodes evaluate to actions, such as "write this PNG here with these
  settings". A separate execute step performs them and never overwrites
  existing files.

### 4. Command line

- Sidecar-aware rendering as described under Decisions.
- Possibly a command that writes the default graph out as a sidecar, so edits
  can be made by hand before the GUI exists.

### 5. Caching

- A node's cache key combines its parameters with its inputs' keys, so changing
  a downstream parameter reuses everything upstream of it.
- Test this by counting evaluations, not by timing.

### 6. Requests

- Requests carry a region, a preview scale and cancellation through the
  evaluator, and nodes declare their locality.
- Add a preview-scale path for raw input. For example, develop a half-size
  image straight from the mosaic by turning each 2×2 block of photosites into
  one pixel. This is also where faster photo loading in the app belongs.

### 7. GUI

- The app runs on the evaluator. It gets a node list, an inspector with a
  control for each parameter, and a viewer showing the selected node.
- Edits save to the sidecar. Autosave comes in milestone 3.

## Conventions

These come from the project's `CLAUDE.md` and practice so far.

- **Hegel property tests over unit tests.** Use as many test cases as fit in
  about 200 ms per test.
- **The UI is tested headlessly** with `egui_kittest`, driving real input
  events and checking rendered pixels.
- **Real photos:** the sample-photo test runs on real raw files when pointed at
  one, and can save renders for inspection.
- **Small, incremental commits:** something simple and evidently correct first,
  then edge cases and optimizations.
