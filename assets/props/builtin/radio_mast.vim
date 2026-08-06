# radio_mast.vim — builtin template for the `RadioMast` feature generator
# (da-param). Meters, Z-up, base at z = 0. The generator binds `height`
# from the zone RON. Part -> material mapping:
#   mast -> MastPole (metal)   arms -> MastCrossarm (metal)
#   beacon -> MastBeacon (red aviation beacon, emissive)
let height = 30.0        # mast height (bound from RadioMast.height_m)

" tapered tube mast — wide guyed base, slender tip
let mast = frustum(0.45, 0.14, height, 12).move(0, 0, height / 2)

" two antenna crossarms at 55% and 80% height, 90 degrees apart in plan
let arms = box(3.2, 0.16, 0.16).move(0, 0, height * 0.55) + box(0.16, 3.2, 0.16).move(0, 0, height * 0.8)

" aviation beacon floating just above the tip
let beacon = sphere(0.25, 12).move(0, 0, height + 0.4)

model mast + arms + beacon
