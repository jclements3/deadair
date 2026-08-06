" Slender ICBM reentry vehicle — fine biconic nose, long slender flanks,
" small flat base. Silhouette read as (r, z): a sharp tip on the axis, a
" gentle biconic flare, then a short base cap back to the axis. lathe()
" revolves it into a narrow, high-fineness-ratio hull.
let hull = bezier(0,18,    0.05,17.5, 0.5,16.0, 0.7,14.0,
                  0.7,14.0, 1.0,9.0,  1.15,4.0,
                  1.15,4.0, 1.2,2.0,  1.2,1.5,
                  1.2,1.5,  0.6,1.5,  0,1.5)
model lathe(hull, 96)
