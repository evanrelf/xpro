# Ideas

A grab bag of ideas. Nothing here is committed.

## Inspiration

- **Modular synthesizers (Eurorack):** small modules you patch together, next
  to "macro" modules like Mutable Instruments' Plaits that hide a lot of
  machinery behind a few well-chosen knobs.
- **Nuke:** point the viewer at any node; every node has a mask input and a
  mix amount; groups of nodes with exposed controls; computes only the region
  that's needed.
- **DaVinci Resolve:** qualifiers and shape windows that produce masks;
  compound nodes; shared nodes and group grades (one look across many clips);
  user-written color transforms (DCTL); scopes.
- **Blender:** node groups; typed, color-coded sockets; frames and reroutes
  that keep big graphs readable.
- **Houdini:** digital assets, which are versioned, packaged subgraphs with
  promoted parameters.
- **Substance Designer:** graphs as parameterized, reusable functions.
- **ComfyUI:** the graph is embedded in each output image, so dropping an
  output back in recovers the edit.
- **darktable / RawTherapee / ART:** references for raw processing algorithms
  and scene-referred workflows.
- **vkdt:** raw processing as a node graph on the GPU. Xpro's closest
  relative.

## Nodes

- **Develop:** demosaic variants, highlight reconstruction, lens corrections
- **Tone and color:** curves, color wheels, channel mixer (B&W with color
  filters), split toning, saturation, LUT apply
- **Detail:** sharpening, local contrast, clarity, dehaze, denoise, frequency
  separation
- **Geometry:** crop, rotate, perspective, resize, seam carving
- **Film and creative:** grain, halation, bloom, film emulation, vignette,
  posterize, dither, halftone, pixel sorting, palette extraction
- **Masks:** gradients, luminance and color ranges, mask math, AI subject and
  sky segmentation
- **Multiple images:** blending, exposure fusion, median stacking (removes
  passers-by), mean stacking (simulated long exposure), focus stacking,
  panoramas
- **Analysis:** histogram, waveform, vectorscope, clipping overlay, seam-energy
  map
- **Outputs:** image files, a baked LUT (export a graph's color transform for
  use elsewhere), contact sheets, diptychs

## Macro nodes

- **Basic:** the familiar Lightroom sliders (exposure, contrast, highlights,
  shadows, whites, blacks, temperature, tint, vibrance) in one node
- **Film:** stock character, grain, halation, fade
- **Punch:** contrast, local contrast and saturation moving together
- **Glow:** bloom and soft highlights
- **Matte:** lifted blacks and muted color

## Features

- Show the raw file's embedded JPEG instantly while the real decode runs
- Fast approximate previews for heavy nodes while you drag, with the full
  render when you let go
- Controls drawn directly on the image: a white-balance eyedropper, handles
  for gradients and crops
- Before/after and A/B comparison in the viewer
- Several output branches in one graph as "virtual copies" of a photo
- A custom-expression or shader node
- Tiled evaluation for fast 100% zoom on large images
