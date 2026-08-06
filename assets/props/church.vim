# church.vim — nave with gable roof, front tower with pyramid spire,
# arched main door, tall side windows.

let nave_w = 8
let nave_d = 16
let nave_h = 5
let roof_h = 3

let tower_w = 4
let tower_h = 12

let yfront = 0 - nave_d / 2
let ytower = yfront - 1            #  tower straddles the front wall
let ytowerface = ytower - tower_w / 2

let nave = box(nave_w, nave_d, nave_h).move(0, 0, nave_h / 2)

# gable roof; add .rotatez(90) if the wedge ridge runs across instead of along
let roof = wedge(nave_w + 0.6, nave_d + 0.6, roof_h).move(0, 0, nave_h + roof_h / 2)

let tower = box(tower_w, tower_w, tower_h).move(0, ytower, tower_h / 2)
let spire = pyramid(tower_w + 0.4, tower_w + 0.4, 4).move(0, ytower, tower_h + 2)

# arched door in the tower face: rectangular cut + half-round top
let doorslab = box(1.6, 0.6, 2).move(0, ytowerface, 1)
let doorarch = cylinder(r = 0.8, h = 0.6, seg = 32).rotatex(90).move(0, ytowerface, 2)
let door = doorslab + doorarch

# tall narrow windows along the right side of the nave, mirrored left
let win = box(0.5, 1, 2.4)
let winrow = win.arrayy(4, 3.2).move(nave_w / 2, 0 - 4.8, 2.8)
let wins = winrow + winrow.mirror("x")

# belfry opening near the top of the tower, front face
let belfry = box(1.4, 0.6, 1.8).move(0, ytowerface, tower_h - 1.6)

model nave + roof + tower + spire - door - wins - belfry
