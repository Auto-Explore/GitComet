# macOS elastic scrolling

Scrollable repository views support rubber-band displacement at the edges of
vertical content on macOS.

`gitcomet-ui-kit::elastic_scroll::ElasticScroll` wraps a scrolling body without
adding a layout node or changing its virtualized row model. Logical offsets stay
clamped. The wrapper applies a resisted vertical displacement during prepaint
inside a fixed clip; column headers and overlay scrollbars remain outside it.
Nested wrappers give the innermost area ownership of precise vertical input.
Gesture release and cancellation reach every stretched wrapper independently of
which area currently owns the pointer.

macOS supplies the normal scrolling and momentum deltas. The wrapper does not
generate another inertial scroll stream. AppKit's public `currentEvent`, `phase`
and `momentumPhase` APIs fill in metadata omitted by the pinned GPUI adapter,
including cancellation. Native metadata is used only when deltas, precision and
phase match the dispatched packet; otherwise the GPUI touch-phase stream is
used. A synthetic packet identical to a retained native event cannot be
distinguished through this API. Smooth mouse-wheel drivers without touch or
momentum phases scroll normally without elasticity. Line wheels and horizontal
scrolling keep the existing handlers.

Finger travel is resisted with a bounded hyperbolic curve (72 logical pixels
maximum displacement). A critically damped spring returns the edge after lift
or momentum impact. A fling throws an edge once per gesture. Inward movement
takes up displacement before moving into the document, so virtualized content
does not reveal an unrendered gap. Holding a stretched edge needs no animation
frames; return animations stop once settled. Reduced Motion, including the
current macOS accessibility preference, suppresses displacement.

History's indexed viewport uses its f64 logical position directly, rather than
the scrollbar's normalized fractions. Linked split-diff columns share visual
displacement and retain their existing logical synchronization. Document,
repository and presentation changes clear their elastic state.

Sidebar sticky rows use the uniform list's measured geometry but paint in a
separate fixed viewport layer. Pinned headers stay anchored during elasticity;
headers that have not reached an anchor move with the rows beneath them.

The wrapper is installed in history, branch and file-browser sidebars, the shared
changed-file lists (including staged and unstaged files), inline and split text
diffs, and plain-text file previews. Specialized image, diagram and terminal
surfaces keep their own gesture handlers.

Tests exercise boundary clamping, reversal, release, cancellation ownership,
momentum, smooth wheels, Reduced Motion, fixed headers, linked columns and
million-row virtualization. App integration includes the indexed history input
path and subpixel movement at two million rows.

The motion is an approximation of macOS elasticity, not an embedded NSScrollView.
The existing tab-strip rendering cost remains a separate source of frame delay
when hundreds of repository tabs are open.
