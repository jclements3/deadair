# shop.vim — single-story corner store: storefront glazing recess, door,
# awning, sign board, parapet roof.

let w = 8
let d = 10
let h = 4

let yfront = 0 - d / 2

let shell = box(w, d, h).move(0, 0, h / 2)

# parapet well in the flat roof
let roofwell = box(w - 0.6, d - 0.6, 0.5).move(0, 0, h - 0.15)

# storefront glazing recess with the entry door cut deeper inside it
let glazing = box(5.6, 0.5, 2.4).move(0, yfront, 1.5)
let door = box(1.2, 0.8, 2.3).move(1.8, yfront, 1.15)

# awning over the storefront and a sign board above it
let awning = wedge(6.4, 1, 0.5).move(0, yfront - 0.5, 2.9)
let sign = box(6, 0.3, 0.8).move(0, yfront - 0.05, 3.5)

# small service door recess at the back
let backdoor = box(1.1, 0.5, 2.1).move(0 - 2.4, d / 2, 1.05)

model shell + awning + sign - roofwell - glazing - door - backdoor
