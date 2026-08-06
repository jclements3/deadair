" sweep -- push a cross-section along a curved spine with a rotation-minimizing
" frame (minimal twist), then cap the ends into a watertight tube. The `path`
" sketch is read as (x, z); a bezier makes a smooth spine. Here a round tube
" follows a C-bend -- the shape of a harp arm / pillar sweep.
model sweep(circle(1.5, 48), bezier(0,-10, 6,-4, 6,4, 0,10))
