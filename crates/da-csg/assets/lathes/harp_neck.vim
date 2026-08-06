" Harp-neck showcase -- a fluted rose section swept along a curved bezier spine
" with a rotation-minimizing frame (no twist). The `bezier` points are read as
" the (x, z) centerline; sweep() rides the section along it and caps the ends.
model sweep(rose(4, 12, 0.35), bezier(0,-15, 3,-6, 8,4, 6,15))
