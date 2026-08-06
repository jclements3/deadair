# gravestone_b.vim — builtin `Cemetery` headstone, variant B (da-param):
# a tall slim tablet on a plinth. Meters, Z-up, base at z = 0.
# Part -> material mapping:
#   slab -> Headstone (concrete/stone)   plinth -> HeadstoneBase (concrete)
let plinth = box(0.8, 0.3, 0.14).move(0, 0, 0.07)
let slab = extrude(roundrect(0.55, 1.1, 0.24, 5), 0.14).rotatex(90).move(0, 0, 0.66)
model plinth + slab
