" Ogive SRBM hull — blunt tangent-ogive nose, cylindrical body, flat base.
" The bezier silhouette is read as (r, z): the nose tip sits on the axis
" (r = 0) at the top, the base closes back to the axis at the bottom, and
" lathe() spins the profile 360 deg into a watertight body of revolution.
let hull = bezier(0,12,   0.3,11.7, 2.0,10.8, 2.0,9.2,
                  2.0,9.2, 2.0,2.2,  2.0,1.6,
                  2.0,1.6, 1.2,1.5,  0,1.5)
model lathe(hull, 96)
