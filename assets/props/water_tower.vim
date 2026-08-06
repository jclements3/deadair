# water_tower.vim — landmark filler for the skyline: four legs, tank, cone cap.
# Round parts kept at seg 32–48 per the manual's boolean-cost rule.

let leg_h = 10
let tank_r = 3
let tank_h = 4
let leg_spread = 2.2

let leg = cylinder(r = 0.25, h = leg_h, seg = 24).move(leg_spread, 0, leg_h / 2)
let legs = leg.polar(4)

let stem = cylinder(r = 0.8, h = 3, seg = 32).move(0, 0, leg_h - 1)

let tank = cylinder(r = tank_r, h = tank_h, seg = 48).move(0, 0, leg_h + tank_h / 2)
let cap = cone(tank_r + 0.2, 1.6).move(0, 0, leg_h + tank_h + 0.8)

model legs + stem + tank + cap
