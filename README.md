# dezoomify-rs

[![Build Status](https://travis-ci.org/lovasoa/dezoomify-rs.svg?branch=master)](https://travis-ci.org/lovasoa/dezoomify-rs)

This is a prototype for a new version of
[dezoomify](https://github.com/lovasoa/dezoomify)
written in [rust](https://www.rust-lang.org/).

The goal of this project is not to replace the traditional dezoomify.
However, it can dezoom even 
very large images, that currently cannot be dezoomed inside a browser
because of memory constraints.

The following dezoomers are currently available:
 - [**zoomify**](#zoomify) supports the popular zoomable image format *Zoomify*.
 - [**IIIF**](#IIIF) supports the widely used International Image Interoperability Framework format.
 - [**Google Arts & Culture**](#google-arts-culture) supports downloading images from
    [artsandculture.google.com](https://artsandculture.google.com/);
 - [**custom**](#Custom) for advanced users.
    It allows you to specify a custom tile URL format.

## Usage instructions

### Download *dezoomify-rs*
First of all, you have to download the application.

 1. Go to the the [latest release page](https://github.com/lovasoa/dezoomify-rs/releases/latest),
 1. download the version that matches your operating system (Windows, MacOS, or Linux),
 1. Extract the binary from the compressed file.
 
On some operating systems, you may have to authorize the application execution
before being able to launch it. See how to do
[in MacOS](https://support.apple.com/kb/ph25088?locale=en_US).


## Dezoomers

### Custom

The custom dezoomer can be used when you know the form of the individual tile URLs.

#### Create a `tiles.yaml` file

You have to generate a [`tiles.yaml`](tiles.yaml) file that describes your image.

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

Then place this file in the same directory as the executable file you downloaded,
launch `dezoomify-rs` in a terminal and when asked, enter `tiles.yaml` as the tile source. 

### Google Arts Culture
In order to download images from google arts and culture, just open 
`dezoomify-rs`, and when asked, enter the URL of a viewing page, such as 
https://artsandculture.google.com/asset/light-in-the-dark/ZQFouDGMVmsI2w 

### Zoomify

You have to give dezoomify-rs an url to the `ImageProperties.xml` file.
If the image tile URLs have the form
`http://example.com/path/to/TileGroup1/1-2-3.jpg`,
then the URL to enter is
`http://example.com/path/to/ImageProperties.xml`.

### IIIF

The IIIF dezoomer takes the URL of an
 [`info.json`](https://iiif.io/api/image/2.1/#image-information) file as input.
You can find this url in your browser's network inspector when loading the image.

## Command-line options

When using dezoomify-rs from the command-line

```
USAGE:
    dezoomify-rs [FLAGS] [OPTIONS] [ARGS]

FLAGS:
        --help       Prints help information
    -l               If several zoom levels are available, then select the largest one
    -V, --version    Prints version information

OPTIONS:
    -d, --dezoomer <dezoomer>        Name of the dezoomer to use [default: auto]
    -h, --max-height <max_height>    If several zoom levels are available, then select the one with the largest width
                                     that is inferior to max-width.
    -w, --max-width <max_width>      If several zoom levels are available, then select the one with the largest width
                                     that is inferior to max-width.

ARGS:
    <input_uri>    Input URL or local file name
    <outfile>      File to which the resulting image should be saved [default: dezoomified.jpg]
```