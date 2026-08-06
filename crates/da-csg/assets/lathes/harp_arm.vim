" Harp-arm showcase -- the ported Clements-49 algorithms working together.
" loft() morphs a limaçon "soundbox foot" cross-section up into a fluted rose
" column (with the DTW correspondence keeping the differing shapes from
" tangling), then a central bore hollows it. Stands Z-up like a real pillar.
let foot  = limacon(2, 7)
let mid   = rose(6, 12, 0.30)
let crown = rose(5, 12, 0.50)
let shell = loft(foot, mid, crown, h = 30)
let bore  = cylinder(r = 2.5, h = 34)
model shell - bore
