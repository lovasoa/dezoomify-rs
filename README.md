# dezoomify-rs

[![Build Status](https://travis-ci.org/lovasoa/dezoomify-rs.svg?branch=master)](https://travis-ci.org/lovasoa/dezoomify-rs)

This is a prototype for a new version of
[dezoomify](https://github.com/lovasoa/dezoomify)
written in [rust](https://www.rust-lang.org/).

The goal of this project is not to replace the traditional dezoomify.
However, it would have the potential of being able to dezoom even 
very large images, that currently cannot be dezoomed inside a browser
because of memory constraints.

## Usage instructions

### Download *dezoomify-rs*
First of all, you have to download the application.

 1. Go to the the [latest release page](https://github.com/lovasoa/dezoomify-rs/releases/latest),
 1. download the version that matches your operating system (Windows, MacOS, or Linux),
 1. Extract the binary from the compressed file.

### Create a `tiles.yaml` file
You have to generate a [`tiles.yaml`](./example.yaml)
file that describes your image.

 1. In a text editor, create an empty plaintext file, and save it under `tiles.yaml`.
 1. Paste the following template to the file, changing it to match your own image.
 
If you need help creating the file, you can follow the [step-by-step tutorial](https://github.com/lovasoa/dezoomify-rs/wiki/Usage-example), that follows a concrete example.

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

### Launch dezoomify-rs

You can now launch the application. In a terminal, type :

```bash
dezoomify-rs tiles.yaml result.jpg
```

where `dezoomify-rs` is the path to the binary you extracted,
`tiles.yaml` is the path to the configuration file you wrote, and
`result.jpg` is the path to the image file that will be generated.
You can change `.jpg` to `.png` to generate a losslessly compressed
(but much larger) file.

The url template string will be evaluated for each pair of coordinates,
following the x and y limits you specified.
This should create a `result.jpg` file containing the dezoomed image.
