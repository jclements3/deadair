# fluted_silo.vim — rose-fluted grain silo with a domed cap.
# Meters, Z-up (vali convention); base sits at z = 0 so the prop stands on
# the ground after da-csg's Y-up conversion. Flute tips reach radius 2*b.
let b     = 1.3          # rose parameter: petal tips reach 2*b = 2.6 m
let h     = 14.0         # barrel height
let hz    = h / 2
let cap_r = 2.62         # dome just covers the flute tips

" fluted barrel: a rose section extruded up, lifted so the base is at z=0
let barrel = extrude(rose(b, 12, 0.22, 64), h).move(0, 0, hz)

" domed cap centered on the top plane; its equator matches the flute tips
let dome = sphere(cap_r, 24).move(0, 0, h)

" unloading chute stub at the base — the feed spill line rats work
let chute = box(1.2, 0.8, 1.0).move(2.6, 0, 0.5)

model barrel + dome + chute
