# Milestone 2 plan

The working plan for [milestone 2, "A graph, headless first"](roadmap.md). It
records the decisions already made and the steps that remain, so work can pick
up without prior context. Read [design.md](design.md) first, and use the terms
in [glossary.md](glossary.md). Update this file as steps land, and fold lasting
decisions into the design doc or glossary as they're settled.

**Done when** you can edit an image file, save the edit, and rerun the same
graph on a different image file from the command line.

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
app also keeps the image at several halved sizes so it can draw sharply at any
zoom.

## Decisions

- **Fine-grained nodes, no monolithic "develop" node.** Each step of raw
  development is its own node, with a little configuration where it makes sense.
  Friendlier, higher-level editing comes later from composites built out of
  these primitives.
- **Built-in nodes are an enum of plain structs**, one variant per node type,
  each struct holding its typed parameters. This keeps the file format simple
  through serialization and keeps matching exhaustive. Third-party nodes, if
  they ever come, can be added as a dynamic variant without changing the rest.
- **A node's inputs live inside the node**, as fields naming the outputs they're
  wired to, next to its parameters, rather than in a separate list of wires.
- **Node IDs are generated, readable and unordered:** the node type in snake
  case plus a short random suffix, like `exposure_k3f9`. Nobody types them by
  hand. The random suffix avoids encoding the order nodes were added. A node can
  also have an optional label, a human-readable name that doesn't have to be
  unique.
- **Each kind of file gets its own extension.** A graph file is `.xpro`. An
  edit file is named after its image file's full filename plus `.xpro-edit`, so
  a raw file and a JPEG with the same name get separate edits:
  `DSC_4362.NEF.xpro-edit`. Don't force different kinds of file into one
  extension.
- **The command line is edit-aware.** Rendering an image file uses its edit
  file if it has one, otherwise a default graph. A flag applies some other graph
  instead, such as a graph file or another image file's edit.
- **Images carry their color space,** starting as the camera's own RGB. The view
  transform only applies the tone curve, so a display-referred image is still
  linear Rec.2020. Converting to sRGB and encoding happen on output.
- **Graph inputs are named.** A Read Raw node names the graph input it reads,
  `image` by default, and evaluating binds each name to an image file.
- **Output paths are templates, for now,** like `{image}.png`, where `{image}`
  is the name of the image file bound to the graph input `image`, without its
  extension. Relative paths resolve against the image file's directory. Other
  ways of naming outputs may come later, so templates stay inside the Write PNG
  node: actions and execution only ever see plain paths.
- **An edit file is a graph file plus one field,** `input`, naming the graph
  input bound to the image file it's next to.
- **Headless first.** The core, file format, evaluator and CLI come before the
  GUI, so the GUI lands on a settled core.

## Steps

Each step is one or a few commits, tested headlessly before moving on.

### 0. Glossary (done)

[glossary.md](glossary.md) holds the canonical terms, and [story.md](story.md)
shows them in use. It's deliberately small: add a term when a step needs one,
with the user's review, rather than in advance.

### 1. Values and nodes, without a graph (done)

- **Value types:** mosaic, scene-referred image and display-referred image.
  A scene-referred image carries its color space, which starts as the camera's
  own RGB and becomes Rec.2020 after conversion. Values also carry the metadata
  later nodes need, such as the as-shot white balance, the camera color matrix
  and the orientation.
- **Node structs,** each with typed parameters and an evaluate function:
  reading a raw file, white balance, demosaic, camera to working space,
  orient, exposure, view transform and a PNG output node. The baseline exposure
  becomes the default for the exposure node in the default graph, no longer a
  hidden step.
- Chain the nodes in code to replace the fixed pipeline. Rendered output must
  stay the same, which makes the refactor easy to verify.

### 2. The graph and its file format (done)

- **The graph:** a map from node IDs to nodes, each with an optional label,
  plus a file format version. Serialize it to RON.
- **Validation:** inputs that refer to missing nodes, cycles, and type
  mismatches, such as a display-referred image wired into exposure.
- **Tests:** property tests that graphs round-trip through the file format, and
  that validation catches each kind of error.

### 3. Evaluate and execute (done)

- An evaluator that computes a node's value by evaluating its inputs first, with
  no caching yet.
- Output nodes evaluate to actions, such as "write this PNG here with these
  settings". A separate execute step performs them and never overwrites
  existing files.

### 4. Command line (done)

- Edit-aware rendering as described under Decisions.
- `xpro new-edit` writes the default graph out as an edit file, so edits can be
  made by hand before the GUI exists.

### 5. Caching (done)

- A node's cache key combines its parameters with its inputs' keys, so changing
  a downstream parameter reuses everything upstream of it.
- Test this by counting evaluations, not by timing.

### 6. Requests (done, except regions)

- Requests carry a region, a preview scale and cancellation through the
  evaluator, and nodes declare their locality.
- Regions and locality are deferred to milestone 3, where tiled zooming first
  needs them. They need a design for learning each node's output size and
  geometry before computing pixels, and for images that cover only part of the
  frame. Cancellation is checked between nodes, not yet within them.
- Add a preview-scale path for reading raw files. For example, develop a
  half-size image straight from the mosaic by turning each 2×2 block of
  photosites into one pixel. This is also where faster loading in the app
  belongs.

### 7. GUI (done)

- The app runs on the evaluator. It gets a node list, an inspector with a
  control for each parameter, and a viewer showing the selected node.
- Edits save to their edit files. Autosave comes in milestone 3.
- The viewer shows any value: a mosaic as gray, and a scene-referred image
  through the default view transform, so it looks like the finished picture.
- Adding, removing and rewiring nodes waits for the canvas in milestone 3.
  Until then, the graph can be changed by editing the edit file by hand.

## Conventions

These come from the project's `CLAUDE.md` and practice so far.

- **Hegel property tests over unit tests.** Use as many test cases as fit in
  about 200 ms per test.
- **The UI is tested headlessly** with `egui_kittest`, driving real input
  events and checking rendered pixels.
- **Real raw files:** one test develops a real raw file when pointed at one,
  and can save renders for inspection.
- **Small, incremental commits:** something simple and evidently correct first,
  then edge cases and optimizations.
