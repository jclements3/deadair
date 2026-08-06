" More scripted math families -- star, superellipse, petals, hypotrochoid.
" Each produces an ordinary 2D sketch, so extrude / revolve / boolean them like
" circle or rect. Here: a star badge next to a squared-off superellipse pad, a
" petalled rose, and a spirograph token.
let badge  = extrude(star(5, 10, 4), 4)
let pad     = extrude(superellipse(9, 6, 4), 4).move(28, 0, 0)
let flower  = extrude(petals(8, 5), 3).move(0, 28, 0)
let token   = extrude(hypotrochoid(10, 3, 5), 3).move(28, 28, 0)
model badge + pad + flower + token
