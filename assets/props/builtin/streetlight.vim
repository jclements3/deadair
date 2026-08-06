# streetlight.vim — builtin template for one lamp unit of `StreetlightRow`
# (da-param). Meters, Z-up, base at z = 0. Part -> material mapping:
#   pole -> StreetlightPole (metal)   arm -> StreetlightArm (metal)
#   head -> StreetlightHead (emissive lamp glass, the NV bloom source)
let pole_h = 4.5

" tapered pole standing on the ground
let pole = frustum(0.11, 0.07, pole_h, 12).move(0, 0, pole_h / 2)

" arm reaching out over the street (+x), slightly drooped
let arm = cylinder(0.05, 1.0, 8).rotatey(96).move(0.5, 0, pole_h - 0.08)

" lamp head hanging at the arm tip
let head = sphere(0.28, 12).move(1.0, 0, pole_h - 0.25)

model pole + arm + head
