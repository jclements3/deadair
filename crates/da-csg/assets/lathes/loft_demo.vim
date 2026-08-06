" loft -- skin a watertight solid through a stack of cross-sections.
" Each positional sketch is one section; `h` spreads them evenly along Z,
" centered on the origin. Sections are resampled to a common vertex count,
" rolled apex-to-apex, and twist-aligned, so even dissimilar shapes (here a
" square base blending up through a circle to a small circular tip) skin
" cleanly. Feed the result to booleans/transforms like any other solid.
model loft(rect(8, 8), circle(5, 64), circle(2, 64), h = 20)
