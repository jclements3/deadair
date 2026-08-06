" Scripted math sections -- limaçon soundbox + fluted rose column.
" limacon(c,b) / cardioid(b) / rose(b) all produce ordinary 2D sketches, so you
" can extrude, revolve, or boolean them like circle/rect/polygon.
let soundbox = extrude(limacon(2, 8), 6)
let column   = extrude(rose(6), 14).move(24, 0, 0)
let bore     = cylinder(r = 3, h = 16).move(24, 0, 0)
model soundbox + (column - bore)
