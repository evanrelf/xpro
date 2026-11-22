# Story

How Xpro is meant to feel in use, told as one picture's trip from the camera to
a finished file. It uses the terms from the [glossary](glossary.md), each in
bold the first time it appears.

You come home from a walk with a card full of **raw files**. Each one is an
**image file** holding the camera sensor's data before development, not yet a
picture you could look at. You open `DSC_4362.NEF` in Xpro.

Xpro starts an **edit** for it: a **graph** with one of its **inputs** bound to
that raw file. A new edit begins from a default graph, a short chain of
**nodes** that develops the raw file the way you'd expect.

The first node reads the raw file and produces a **mosaic**, with one color
sample per photosite. A White Balance node scales the mosaic's color channels,
and a Demosaic node fills in the colors each photosite didn't measure, turning
the mosaic into a **scene-referred image**: an **image** whose numbers are
proportional to the light in the scene. An Exposure node brightens it, by an
amount set with its stops **parameter**. Then a View Transform node rolls off
the highlights and turns the scene-referred image into a
**display-referred image**, one that describes how the picture should look on a
screen. Last comes an **output node** that writes a PNG. White Balance,
Demosaic, Exposure and View Transform are **node types**, and an edit can hold
several nodes of the same node type, such as a second Exposure node for a
finishing touch.

Every node has **ports**: inputs, where **values** come in, and **outputs**,
where they leave. A **wire** carries a value from one node's output to another
node's input. Each port has a **value type**, so Xpro won't let you wire a
display-referred image into an Exposure node's input. Exposure multiplies light,
which only makes sense for a scene-referred image. To lighten the picture after
the View Transform, you'd reach for a Brightness node instead, which works on a
display-referred image and adjusts how bright things look rather than how much
light there was.

As you drag the stops parameter, Xpro **evaluates** the graph, computing each
value you need to see, without side effects. The graph is pure, so the same
inputs and parameters always give the same values. Even the output node only
produces a value: an **action** that says "write this display-referred image to
`DSC_4362.png`". Nothing touches your disk until you ask Xpro to **execute** it,
and it never overwrites a file you already have.

Your edit is saved in an **edit file**, `DSC_4362.NEF.xpro-edit`, right next to
the raw file, which Xpro never modifies. The edit file is plain text. Open it in
a text editor and you'll see each node listed under its **node ID**, like
`exposure_k3f9`, which Xpro generated so you never have to make one up. Give
that node a **label** such as "Brighten shadows", and the label shows up
wherever the node does.

You like the look enough to use it again, so you save the graph on its own as a
**graph file**, `portra-look.xpro`. A graph doesn't refer to any particular
image file, only to inputs waiting for one, so you can apply `portra-look.xpro`
from the command line to every other raw file on the card.

A graph isn't limited to one input. Another graph might take an image and a
depth map of the same scene as two inputs, with the depth map wired into a
sharpening node so the background stays soft.
