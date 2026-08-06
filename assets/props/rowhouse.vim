# rowhouse.vim — 3-story flat-roof townhouse with parapet.
# Butt several together with VimProp yaw 0/180 to make a street wall.

let w = 7
let d = 9
let floors = 3
let fh = 3.2
let h = floors * fh

let yfront = 0 - d / 2

let shell = box(w, d, h).move(0, 0, h / 2)

# parapet: sink a shallow well into the roof slab
let roofwell = box(w - 0.6, d - 0.6, 0.6).move(0, 0, h - 0.2)

# upper-floor window grid, front face (3 across x 2 floors)
let win = box(1.1, 0.5, 1.6)
let row = win.arrayx(3, 2.2).move(0 - 2.2, yfront, 0)
let grid = row.arrayz(2, fh).move(0, 0, fh + 1.7)

# ground floor: door on the right, two windows left of it
let door = box(1.2, 0.5, 2.2).move(2.2, yfront, 1.1)
let gwin = win.move(0 - 2.2, yfront, 1.6) + win.move(0, yfront, 1.6)

# back face gets the plain grid mirrored, all three floors
let backrow = row.arrayz(floors, fh).move(0, 0, 1.7)
let back = backrow.mirror("y")

model shell - roofwell - grid - door - gwin - back
