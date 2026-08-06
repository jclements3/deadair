" Blunt boat-tail hull — rounded blunt nose, straight cylindrical body, and a
" tapering boat-tail down to a small base. Silhouette read as (r, z): the nose
" rounds off the axis at the top, the tail necks back in, and a short base cap
" closes it on the axis. lathe() revolves the profile into a solid.
let hull = bezier(0,10,    1.4,10.0, 1.6,9.3, 1.6,8.5,
                  1.6,8.5,  1.6,3.5,  1.6,3.0,
                  1.6,3.0,  1.5,1.2,  1.0,0.6,
                  1.0,0.6,  0.7,0.5,  0,0.5)
model lathe(hull, 96)
