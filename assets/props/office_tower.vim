# office_tower.vim — 10-story tower with a recessed glazing band wrapping
# all four faces on every floor, lobby recess, mechanical penthouse.

let w = 12
let d = 12
let floors = 10
let fh = 3.4
let h = floors * fh

let yfront = 0 - d / 2

let core = box(w, d, h).move(0, 0, h / 2)

# one wraparound band = oversize slab minus its middle; array it per floor
let band_out = box(w + 0.4, d + 0.4, 1.8)
let band_in = box(w - 0.8, d - 0.8, 2.2)
let band = band_out - band_in
let bands = band.arrayz(floors, fh).move(0, 0, 1.8)

# lobby entrance
let lobby = box(3.2, 0.8, 2.8).move(0, yfront, 1.4)

# penthouse / mechanical box on the roof
let penthouse = box(6, 6, 2.5).move(0, 0, h + 1.25)

model core + penthouse - bands - lobby
