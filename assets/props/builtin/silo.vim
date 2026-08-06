# silo.vim — builtin template for the `Silo` feature generator (da-param).
# Meters, Z-up, base at z = 0. The generator binds `radius` and `height`
# from the zone RON (vim_with_params), then maps part names to materials:
#   barrel -> SiloBarrel (sheet metal)   dome -> SiloDome (thin metal roof)
#   chute  -> SiloChute  (sheet metal)
let radius = 4.0         # barrel radius (bound from Silo.radius_m)
let height = 18.0        # barrel height (bound from Silo.height_m)

" barrel: a lathe silhouette (r, z) — flared base skirt, near-straight wall,
" shoulder closing to the axis at the top. Starts and ends on r = 0 so the
" lathe is watertight.
let sil = bezier(0,0,  radius*0.7,0, radius*1.06,0.1, radius*1.06,0.3,  radius*1.03,1.2, radius,height*0.6, radius,height - radius*0.5,  radius,height - radius*0.15, radius*0.55,height, 0,height,  steps = 4)
let barrel = lathe(sil, 28)

" domed cap seated on the shoulder — separate part so it can carry the
" sky-exposed thin-metal thermal state (reads below ambient on clear nights)
let dome = sphere(radius * 0.95, 16).move(0, 0, height)

" unloading chute stub at the base — the feed spill line rats work
let chute = box(1.4, 0.8, 1.0).move(radius + 0.6, 0, 0.5)

model barrel + dome + chute
