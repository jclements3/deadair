# warehouse.vim — 12x20 m hall with a barrel-vault roof, two roll doors,
# clerestory window strips on both long sides.

let w = 12
let d = 20
let wall_h = 5
let r = w / 2

let yfront = 0 - d / 2

let hall = box(w, d, wall_h).move(0, 0, wall_h / 2)

# barrel vault: cylinder laid along y, keep only the top half
let barrel = cylinder(r = r, h = d, seg = 48).rotatex(90).move(0, 0, wall_h)
let upper = box(w, d, r).move(0, 0, wall_h + r / 2)
let vault = barrel * upper

# two roll-door recesses on the front gable wall
let rolldoor = box(3.4, 0.5, 4)
let doors = rolldoor.move(0 - 3, yfront, 2) + rolldoor.move(3, yfront, 2)

# high window strip on the right side wall, mirrored to the left
let strip = box(0.5, 1.6, 1).arrayy(5, 3.4).move(w / 2, 0 - 6.8, 3.8)
let wins = strip + strip.mirror("x")

model hall + vault - doors - wins
