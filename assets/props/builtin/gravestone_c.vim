# gravestone_c.vim — builtin `Cemetery` headstone, variant C (da-param):
# a low wide marker. Meters, Z-up, base at z = 0.
# Part -> material mapping: slab -> Headstone (concrete/stone).
let slab = extrude(roundrect(0.9, 0.6, 0.18, 4), 0.18).rotatex(90).move(0, 0, 0.3)
model slab
