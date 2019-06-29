# dezoomify-rs

[![Build Status](https://travis-ci.org/lovasoa/dezoomify-rs.svg?branch=master)](https://travis-ci.org/lovasoa/dezoomify-rs)

This is a prototype for a new version of
[dezoomify](https://github.com/lovasoa/dezoomify)
written in [rust](https://www.rust-lang.org/).

The goal of this project is not to replace the traditional dezoomify.
However, it would have the potential of being able to dezoom even 
very large images, that currently cannot be dezoomed inside a browser
because of memory constraints.

The usage of the current version of the tool is very simple.
You first have to generate a [`tiles.yaml`](./example.yaml)
file in the following format:

```yaml
# The url of individual tiles, where {{ expressions }} will be evaluated using the variables below
url_template: "http://www.asmilano.it/fast/iipsrv.fcgi?deepzoom=/opt/divenire/files/./tifs/05/63/563559.tif_files/13/{{x/tile_size}}_{{y/tile_size}}.jpg"

variables:
  # The x position of tiles goes from 0 to the image width with an increment of the tile width
  - name: x
    from: 0
    to: 7520 # Image width
    step: 256 # Tile width

  - name: y
    from: 0
    to: 6000 # Image height
    step: 256 # Tile height

  - name: tile_size
    value: 256
```

Then call

```
dezoomify-rs tiles.yaml result.jpg
```

The url template string will be evaluated for each pair of coordinates,
following the x and y limits you specified.
This should create a `result.jpg` file containing the dezoomed image.