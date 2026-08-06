# apartment_block.vim — 5-story slab block with window grid front + back,
# entrance recess, rooftop stair head.

let w = 14
let d = 11
let floors = 5
let fh = 3
let h = floors * fh

let yfront = 0 - d / 2

let shell = box(w, d, h).move(0, 0, h / 2)

# window grid: 5 across, every floor, front face — mirrored to the back
let win = box(1.2, 0.5, 1.5)
let row = win.arrayx(5, 2.4).move(0 - 4.8, yfront, 0)
let front = row.arrayz(floors, fh).move(0, 0, 1.6)
let cuts = front + front.mirror("y")

# double-door entrance, cut a bit deeper than the window recesses
let entrance = box(2, 0.7, 2.4).move(0, yfront, 1.2)

# rooftop stair/lift head
let stairhead = box(3, 2.5, 2.2).move(3, 2, h + 1.1)

model shell + stairhead - cuts - entrance
