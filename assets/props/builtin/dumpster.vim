# dumpster.vim — builtin template for one dumpster of `DumpsterRow`
# (da-param). Meters, Z-up, base at z = 0. Part -> material mapping:
#   body -> Dumpster (metal)   lid -> DumpsterLid (metal)
" chamfered steel body, 2.0 x 1.3 footprint, 1.4 high
let body = box(2.0, 1.3, 1.4).move(0, 0, 0.7).chamfer(0.06)

" lid propped open, hinged along the back (-y) edge
let lid = box(2.05, 1.35, 0.06).rotatex(-34).move(0, 0.5, 1.5)

model body + lid
