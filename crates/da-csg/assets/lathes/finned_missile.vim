" Finned missile -- a cylindrical body with a conical nose, four cruciform
" tail fins, and four smaller forward canards. Built by unioning primitives:
" the body + nose form the hull, then each fin/canard is a thin box swept out
" from the axis and rotated 90 deg around Z into a cruciform set. A model with
" real hard edges like this shows off orthographic multiview + section hatch
" far better than a smooth body of revolution.
let body = cylinder(r = 0.5, h = 5.0)
let nose = cone(r = 0.5, h = 1.6).move(0, 0, 3.3)
let fin  = box(0.09, 1.2, 1.5).move(0, 1.0, -1.6)
let can  = box(0.06, 0.5, 0.6).move(0, 0.7, 2.0)
model body + nose + fin + fin.rotatez(90) + fin.rotatez(180) + fin.rotatez(270) + can + can.rotatez(90) + can.rotatez(180) + can.rotatez(270)
