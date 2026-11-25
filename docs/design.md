# Design

Xpro is an experimental digital darkroom in which **an edit is a node graph**.
Instead of a fixed stack of sliders, you compose transformations on a visual
canvas: white balance, tone, resizing, film looks, and stranger things like
seam carving. It works the way compositors like Nuke, Fusion or Blender do, but
for still photographs.

Xpro is a toolbox for fun, a place where photography and programming meet. It
isn't trying to replace anyone's daily-driver photo editor. Think of it as the
photography equivalent of a modular synthesizer: small modules patched
together, alongside bigger, opinionated modules that get you somewhere good
quickly.

For a feel of Xpro in use, read the [story](story.md). Its terms are defined in
the [glossary](glossary.md).

## Design values

When tradeoffs come up, these come first.

### Fast feedback

Changes should show up immediately, apart from genuinely heavy computation.
Adjusting a parameter should feel like turning a knob, not like submitting a
job. Very roughly:

| While interacting | Feels |
|---|---|
| 120 fps | Amazing |
| 60 fps | Great |
| 30 fps | Totally fine |
| 5 fps | Tolerable |
| 1 fps | As slow as anything should be |

These describe the *first* response. A quick, lower-resolution result followed
by a full-quality render a few seconds later is fine. A blank or frozen screen
isn't.

### Fearless editing

Experimenting should never cost anything. Undo and redo cover everything.
In-progress work is saved quietly in the background, so nothing is lost.
Nothing the user owns, such as an exported image, is overwritten unless they
explicitly ask.

### Easy things easy, hard things possible

A basic edit should be as easy as in Lightroom. The graph's power is available
when you want it and stays out of the way when you don't.

### Batteries included, at two levels

- **Primitives:** low-level building blocks (channel mixing, curves, math,
  blurs) that use the standard terms of art.
- **Macro nodes:** higher-level, opinionated nodes named for what they *do*,
  not how. A few well-chosen controls stand in for a lot of underlying
  machinery, even when that means departing from the terms of art.

A macro node is just a node. It has no special status.

### Crafted, not assembled

Xpro should feel like one polished, cohesive product, not a pile of features.
The visual design can be simple, but it must be consistent, readable and
intuitive: clean typography, restrained color, and one visual language shared
by every node. Graphs stay legible as they grow, never a rat's nest of
crossing wires. The touchstones are Teenage Engineering and Apple: distilled
and a pleasure to use. Not a Bloomberg Terminal, and no "programmer art".
Modular, yes, but modular the way Teenage Engineering would build it.

## Goals

- **Raw file in, image files out.** The graph *is* the edit.
- **See any point in the pipeline.** Every node shows a thumbnail of its
  result, and a large viewer shows whichever node is selected.
- **Exporting is done by nodes.** Export settings live in the graph as output
  nodes, not in a modal dialog, and a graph can have any number of them.
- **Headless as well as interactive.** Any graph can run from the command line.

## Non-goals

- **Competing with Lightroom or Capture One on raw rendering.** The bar is
  "recognizably right".
- **Library management.** No catalogs, ratings or collections. The filesystem
  is the library.
- **Video, or anything temporal.**
- **Non-photographic data** (astro, scientific imaging). Xpro may assume
  ordinary camera photographs.

## Not planned, but not precluded

These aren't planned, but designs must not rule them out:

- Painted or brush masks
- Third-party nodes (scripting, WASM)
- Versioned node types, so old graphs render the same after a node changes
- A viewer that is itself a node in the graph
- Working color spaces other than the default
- HDR, in any of the senses photographers use the term

## Concepts

### Graphs and edits

A **graph** is a directed acyclic graph of nodes. It's pure: the same inputs
and parameters always produce the same values. A graph's inputs, such as the
image files it develops, aren't part of the graph itself, so the same graph can
be applied to any image file. An **edit** is a graph with one of its inputs
bound to a particular image file. It's stored as human-readable text in an
**edit file** next to that image file, and the image file is never modified.

The **viewer** is part of the UI, not of the graph. It shows the selected
node's result, or a pinned one.

### Evaluating vs. executing

Every node is a pure function, including output nodes. **Evaluating** a graph
computes values and has no side effects. An output node evaluates to a
*description* of an action, such as "write this image here, with these
settings". **Executing** carries those actions out, and only happens when the
user asks: a click in the app, or a command-line run. Editing never touches
the user's files as a side effect.

### Typed ports

Nodes behave like functions, with typed inputs, outputs and parameters. Only
ports with compatible value types can be wired together. Converting between
types is done by explicit **cast nodes** that are visible in the graph, never by
silent coercion. Some nodes are generic. Crop, for example, works on any kind of
image and returns the same kind.

The core value types are *scene-referred image*, *display-referred image* and
*mask*. Others (numbers, colors, curves, LUTs) can be added as needed.

### Scene-referred and display-referred images

Raw data measures light. Values are proportional to the light in the scene,
with no upper limit. That's **scene-referred** data. Screens and image files
need **display-referred** data: values that describe how bright something
should *look* within a display's range, with highlights already rolled off. A
**view transform** (tone mapping) converts the first into the second. It's the
most important cast node.

Some operations are only correct on one side of that step:

- **Scene-referred:** exposure and white balance (they're multiplications of
  light), physically plausible blur, and averaging images together.
- **Display-referred:** posterizing, dithering, and other operations that
  assume a bounded range.

Mixing them up causes visible artifacts, which is why the distinction is part
of the type system.

### Color

Every image carries its color space as metadata. The default working space is
linear and wide-gamut (Rec.2020), but that's a setting, not an assumption
baked into the code. A node whose results depend on a particular color space,
or a class of them (for example, one with plenty of dynamic range), says so in
its type. A mismatch shows up as a warning or an error, never as a silent
conversion.

### Resolution independence

This is a guiding principle, not a hard rule. Where it makes sense, parameters
are relative to the image (for example, a blur radius as a fraction of the
image's size), so the same graph works on any camera's files and on
reduced-size previews. Some things really are tied to physical size, though.
Seam carving works on pixels, and sharpening depends on how big the result
will be shown, whether that's an output node's pixel dimensions or a print on a
gallery wall. Those use whatever units fit.

### Locality

Each node knows which part of its input it needs in order to produce a given
part of its output. That's what lets the evaluator compute only the region
that's needed, render cheap previews, and combine chains of simple nodes.
Common cases:

- **Pointwise:** each output pixel depends on one input pixel (exposure,
  curves).
- **Neighborhood:** it depends on nearby pixels (blur, sharpening).
- **Geometric:** it depends on input pixels at locations the node can compute
  (crop, rotate, resize).
- **Global:** it depends on the whole image (auto exposure, histogram
  operations, seam carving).

Global nodes may look different at preview size than at full size.

### Geometry

Nodes that change geometry (crop, rotate, resize, seam carving) record how
their output pixels map back to the original image. Anything anchored to image
content, such as a mask, can then stay aligned through any chain of geometry
changes.

### Masks (later)

Any node whose output has the same shape as its input can take an optional
mask and a mix amount. The framework provides this once for all such nodes,
rather than each node implementing it. A mask is anchored either to the
**frame** (gradients and vignettes re-fit whatever image they're applied to)
or to the **content** (painted or file-based masks follow the content through
geometry changes).

### Composites (later)

A **composite** is a subgraph packaged as a single node, with its own
interface of exposed parameters and ports. A **preset** is a saved composite.
A preset might be copied into an edit, or linked so that changing the preset
updates every edit that uses it. Composites would also let built-in macro
nodes ship as graphs of primitives that you can unpack when you outgrow them.

## Architecture principles

- **The core doesn't depend on the UI.** The GUI and the CLI are thin shells
  over the same core. If something can't be done headlessly, it's in the wrong
  layer.
- **The framework exists for interop and abstraction.** A single typed
  interface lets any node be wired to any compatible node without knowing
  anything about it. Behind that interface, a node might be a few lines of
  straightforward code, or a heavily optimized SIMD or GPU implementation. That
  complexity stays sealed inside the node and never spreads across the app.
- **The framework doesn't get in the way of performance.** The interface is
  shaped around locality, so the evaluator can exploit it. Helpers make the
  common kinds of node easy to write, and nodes that need low-level access
  (pixel memory, the requested region, threads, the GPU) can get it. The GPU
  is used where it helps, but it's never required.
- **Evaluation is incremental and interaction-aware.** Only what a change
  affects gets recomputed. Only what's visible gets computed, at the
  resolution it's shown. Independent work runs in parallel, and what the user
  is looking at comes first. Outdated work is cancelled, and a fast
  approximation comes first, then gets refined. The UI never waits on
  evaluation.
- **Static where possible, dynamic where necessary.** Built-in nodes are
  ordinary Rust types with typed parameters, inputs and outputs. Dynamic
  checks happen only where they must: wiring made at runtime, and the
  interfaces of user-built composites.
- **Human-readable files.** Graphs are serialized with serde to RON, text that
  can be read, diffed and edited by hand. Files carry a format version so they
  can be migrated.
- **Raw development is ordinary nodes.** Decoding, demosaicing and camera
  color conversion are regular nodes, not a hidden stage. So supporting a
  different sensor type (Bayer vs. X-Trans) just means a different node.

## Open questions

Large design questions that have been deliberately postponed:

1. **Composites and presets.** How are they created and named? How are their
   interfaces defined? How exactly do linking and nesting work?
2. **Anchoring through geometry.** The frame/content anchor model for masks is
   a direction, not yet a proven design. In particular, it's unclear how
   content-anchored masks behave through operations that can't be cleanly
   inverted, like seam carving. Relative parameters raise the same question:
   after a crop, is a vignette or grain size relative to the original image or
   to the cropped image?
3. **Edits with several inputs.** An edit file belongs to one image file, so
   where does an edit that merges several image files (exposure fusion,
   stacking) live, and how does it refer to its other inputs?
