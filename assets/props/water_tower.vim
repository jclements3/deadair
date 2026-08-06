# water_tower.vim — lathed tank on four legs with a center riser pipe.
# Meters, Z-up (vali convention); leg bases sit at z = 0 so the tower
# stands on the ground after da-csg's Y-up conversion.
let leg_h = 9.0
let leg_z = leg_h / 2

" one leg, moved out to radius 1.4 and up onto the ground, then a polar
" array makes all four around the Z axis
let leg  = cylinder(0.15, leg_h, 12).move(1.4, 0, leg_z)
let legs = leg.polar(4)

" center riser pipe reaching up into the tank bowl
let riser = cylinder(0.25, 8.5, 16).move(0, 0, 4.25)

" tank silhouette as a bezier (r, z): starts and ends on the axis (r = 0)
" so the lathe closes watertight — bowl bottom flares out, dome top closes
let sil  = bezier(0,8.0,  1.6,8.0, 2.2,8.6, 2.2,9.4,  2.2,10.4, 1.4,11.6, 0,11.8, steps = 8)
let tank = lathe(sil, 24)

model legs + riser + tank
