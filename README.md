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
 - [**deepzoom**](#DeepZoom) supports Microsoft's *DZI* format (Deep Zoom Image),
 that is often used with the seadragon viewer.
 - [**IIIF**](#IIIF) supports the widely used International Image Interoperability Framework format.
 - [**Google Arts & Culture**](#google-arts-culture) supports downloading images from
    [artsandculture.google.com](https://artsandculture.google.com/);
 - [**generic**](#Generic) For when you know the format of the tile URLs.
 - [**custom**](#Custom-yaml) for advanced users.
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

### DeepZoom

The DeepZoom dezoomer takes the URL of a `dzi` file as input.
You can find this url in your browser's network inspector when loading the image.
If the image tile URLs have the form
`http://test.com/y/xy_files/1/2_3.jpg`,
then the URL to enter is
`http://test.com/y/xy.dzi`.

### Generic

You can use this dezoomer if you know the format of tile URLs.
For instance, if you noticed that the URL of the first tile is 

```
http://example.com/my_image/image-0-0.jpg
```

and the second is 

```
http://example.com/my_image/image-1-0.jpg
```

then you can guess what the general format will be, and give dezoomify-rs
the following:

```
http://example.com/my_image/image-{{X}}-{{Y}}.jpg
```

### Custom yaml

The custom dezoomer can be used when you know the form of the individual tile URLs,
as well as some meta-informations about the file.

In order to use this dezoomer, you'll need to create a `tiles.yaml` file.
See: [Usage example for the custom YAML dezoomer](https://github.com/lovasoa/dezoomify-rs/wiki/Usage-example-for-the-custom-YAML-dezoomer).

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
    -d, --dezoomer <dezoomer>          Name of the dezoomer to use [default: auto]
    -h, --max-height <max_height>      If several zoom levels are available, then select the one with the largest height
                                       that is inferior to max-height.
    -w, --max-width <max_width>        If several zoom levels are available, then select the one with the largest width
                                       that is inferior to max-width.
    -n, --num-threads <num_threads>    Degree of parallelism to use. At most this number of tiles will be downloaded at
                                       the same time.
    -r, --retries <retries>            Number of new attempts to make when a tile load fails before abandoning
                                       [default: 1]

ARGS:
    <input_uri>    Input URL or local file name
    <outfile>      File to which the resulting image should be saved [default: dezoomified.jpg]
```