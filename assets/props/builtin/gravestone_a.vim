# gravestone_a.vim — builtin `Cemetery` headstone, variant A (da-param):
# the classic round-shouldered tablet. Meters, Z-up, base at z = 0.
# Part -> material mapping: slab -> Headstone (concrete/stone).
" a 0.7 x 0.9 roundrect extruded 0.16 thick, stood upright
let slab = extrude(roundrect(0.7, 0.9, 0.12, 4), 0.16).rotatex(90).move(0, 0, 0.45)
model slab
