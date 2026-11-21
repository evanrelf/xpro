# Roadmap

Each milestone should be satisfying on its own: stop at any of them and there's
something real to use.

**Current focus: basic nodes, composed directly.** No abstractions that bundle,
name or hide subgraphs yet.

## 1. See a raw photo (done)

Open a raw file, decode and demosaic it, convert it to the working space, apply
a basic view transform, and show it with zoom and pan.

**Done when** your own photos look recognizably right.

## 2. A graph, headless first

The typed graph model, evaluation and caching, and sidecar files. Add a handful
of basic nodes, from raw input through white balance, exposure and a view
transform to a file output. The GUI is minimal: a node list, an inspector and
the viewer. The CLI runs a graph. The evaluator is designed for regions,
preview scales and cancellation from the start, even before the canvas uses
them.

**Done when** you can edit a photo, save it, and rerun the same graph on a
different photo from the command line.

## 3. The canvas

A visual node editor with type-checked wiring, thumbnails on nodes, and a
viewer that follows the selection. Evaluation runs in the background with fast
previews, and undo, redo and autosave make experimenting safe.

**Done when** adjusting a parameter feels like turning a knob.

## 4. More nodes

Grow the toolbox at both levels: primitives (tone, color, B&W, geometry, seam
carving, LUTs) and macro nodes (a Lightroom-style Basic node, film looks). This
milestone never really ends.

## Later (order to be decided)

- **Masks**
- **Multiple source images:** stacking, exposure fusion, layouts
- **Composites and presets**, including linked presets
