#!/bin/sh
# Entropy for a design direction.
#
# A model cannot be random on request: asked for something unique it returns
# the most probable version of "unique", which is why four runs of the same
# prompt produce four near-identical purple gradients. The variety has to come
# from outside. This prints a string; the mapping from string to palette, type
# and density is recorded in docs/slices/08-tour.md, and the string itself
# never appears in the product.
set -eu
head -c 24 /dev/urandom | base64 | tr -d '=\n'
echo
