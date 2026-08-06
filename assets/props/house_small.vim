# house_small.vim — 6x8 m cottage: gable roof, chimney, door + window recesses.
# Base at z = 0, meters, Z-up.

let w = 6
let d = 8
let wall_h = 3
let roof_h = 2.2

let nx = 0 - w / 2          #  left/right face
let yfront = 0 - d / 2      #  front face

let walls = box(w, d, wall_h).move(0, 0, wall_h / 2)

# gable roof with a small eave overhang; if the ridge runs the wrong way
# in your build, add .rotatez(90)
let roof = wedge(w + 0.6, d + 0.6, roof_h).move(0, 0, wall_h + roof_h / 2)

let chimney = box(0.6, 0.6, 1.8).move(1.6, 2.2, wall_h + roof_h - 0.2)

# front door recess (cut straddles the wall face so the boolean is clean)
let door = box(1.1, 0.5, 2.1).move(0, yfront, 1.05)

# window recesses on the right face, mirrored to the left
let win = box(0.5, 1.2, 1.2)
let win_row = win.arrayy(3, 2.4).move(w / 2, 0 - 2.4, 1.6)
let wins = win_row + win_row.mirror("x")

model walls + roof + chimney - door - wins
