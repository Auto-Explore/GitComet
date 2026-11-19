# macOS elastic scrolling

Ordinary lists, tab strips, documents and editors support rubber-band
displacement at their scroll boundaries on macOS. The inventory and integration
constraints are in [the coverage audit](macos-elastic-scroll-coverage.md).

`gitcomet-ui-kit::elastic_scroll::ElasticScroll` wraps a scrolling body without
adding a layout node or changing its virtualized row model. Logical offsets stay
clamped. The wrapper translates prepaint coordinates inside a fixed clip;
headers, borders, query fields, action controls and overlay scrollbars remain
outside it. Vertical and horizontal wrappers route a precise gesture to the
innermost compatible axis. The initial axis remains locked through small
mixed-axis packets. Release and cancellation reach stretched wrappers even
when the pointer moves into another area.

macOS supplies normal scrolling and momentum. The wrapper does not generate
another inertial scroll stream. AppKit's public `currentEvent`, `phase` and
`momentumPhase` APIs fill in metadata omitted by the pinned GPUI adapter,
including cancellation. Native metadata is used only when deltas, precision and
phase match the dispatched packet; otherwise the GPUI touch-phase stream is
used. A synthetic packet identical to a retained native event cannot be
distinguished through this API. Smooth mouse-wheel drivers without touch or
momentum phases scroll normally without elasticity. Line wheels retain their
existing handlers. Non-macOS and deterministic hosts bypass the wrapper.

Finger travel is resisted with a bounded hyperbolic curve (72 logical pixels
maximum displacement). A critically damped spring returns the edge after lift
or momentum impact. A fling throws an edge once per gesture. Inward movement
takes up displacement before moving into the document, so virtualized content
does not reveal an unrendered gap. Holding a stretched edge needs no animation
frames; return animations stop once settled. Reduced Motion, including the
current macOS accessibility preference, suppresses displacement.

History's indexed viewport uses its f64 logical position directly, rather than
normalized scrollbar fractions. `ScrollHandle`, `UniformListScrollHandle` and
`ListState` adapters preserve fractional positions and the original extents.
Partially measured lists keep their native interior scrolling; their provisional
scrollbar maximum is not treated as a real boundary. Child-measured content
growth requests one cleanup frame when a held edge becomes interior. Linked
columns compare the input master's model, and resize cancellation occurs between
frames so followers never disagree about the displacement for a painted frame.
`tracked_vertical` and `tracked_horizontal` retain a scoped handle for implicit
overflow containers across renders.

`motion_state` exposes one surface's spring to its owner and retains automatic
cancellation on logical position, extent or viewport changes. `shared_motion`
links visual displacement across synchronized columns; their handles retain
independently clamped logical offsets. A shorter column may reach its trailing
edge before a longer follower. The follower must not cancel the shared spring
merely because its own logical offset is still interior. Owners reset shared
state when documents or presentation modes change.

Sidebar and grouped-file headers use measured list geometry but paint in a
fixed sibling overlay. Both pinned and naturally positioned section headers
remain fixed during elasticity, at either end of the list; their rows stretch
below them. Scroll input passes through header chrome to the list. Header
activation uses logical sticky state rather than elastic displacement.

The shared repository picker preserves windowing and keeps its query, sorting
controls and scrollbar fixed. Tab strips stretch horizontally while the
repository strip's trailing add control stays fixed. Programmatic tab reveal
clears displacement. Nested Markdown code and table blocks stretch horizontally
inside their fixed shells, leaving vertical gestures to the document.

Text editing must preserve caret reveal, selections, IME geometry and line-number
alignment. Scroll painting is separate from the logical offsets used by editing;
programmatic reveal and editing clear stale displacement. Terminal content,
image cameras and diagram cameras retain their specialized gesture handlers.
The terminal's ordinary tab strip is elastic.

The motion approximates macOS elasticity; the body remains a GPUI element rather
than an embedded NSScrollView. Existing tab-strip rendering cost when hundreds
of repositories are open is independent of the spring.
