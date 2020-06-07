# [dezoomify-rs](https://lovasoa.github.io/dezoomify-rs/)

[![Continuous Integration](https://github.com/lovasoa/dezoomify-rs/workflows/Continuous%20Integration/badge.svg)](https://github.com/lovasoa/dezoomify-rs/actions)

**dezoomify-rs** is a tiled image downloader.
Some webpages present high-resolution zoomable images without a way to download them.
These images are often *tiled*: the original large image has been split into smaller individual image files called tiles.
The only way to download such an image is to download all the tiles separately and then stitch them together.
This process can be automated by a tiled image downloader.

The most common tiled image downloader is probably [**dezoomify**](https://ophir.alwaysdata.net/dezoomify/dezoomify.html),
an online tool which is very easy to use.


The goal of this project is not to replace the traditional dezoomify.
However, some images are so large that they can't be efficiently downloaded and displayed inside a web browser.
Other times, a website tries to protect its tiles by refusing access to them when certain 
[HTTP headers](https://en.wikipedia.org/wiki/List_of_HTTP_header_fields) are not set to the right values.
**dezoomify-rs** is a desktop application for Windows, MacOs and linux that does not have the same limitations as the online zoomify.
dezoomify-rs also lets the user choose between
[several image formats](https://github.com/image-rs/image#21-supported-image-formats),
whereas in *dezoomify*, you can only save the image as *PNG*.

dezoomify-rs supports several zoomable image formats, each backed by a dedicated *dezoomer*.
The following dezoomers are currently available:
 - [**Google Arts & Culture**](#google-arts-culture) supports downloading images from
    [artsandculture.google.com](https://artsandculture.google.com/);
 - [**zoomify**](#zoomify) supports the popular zoomable image format *Zoomify*.
 - [**deepzoom**](#DeepZoom) supports Microsoft's *DZI* format (Deep Zoom Image),
 that is often used with the seadragon viewer.
 - [**IIIF**](#IIIF) supports the widely used International Image Interoperability Framework format.
 - [**Zoomify PFF**](#zoomify-pff) supports the old zoomify single-file image format.
 - [**Krpano**](#krpano) supports the [krpano](https://krpano.com/home/) panorama viewer
 - [**generic**](#Generic) For when you know the format of the tile URLs.
 - [**custom**](#Custom-yaml) for advanced users.
    It allows you to specify a custom tile URL format.

## Screenshots

![dezoomify-rs screenshot](https://user-images.githubusercontent.com/552629/83974627-1cad7000-a8ef-11ea-865b-e1ea0beacac5.gif)

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

### Zoomify PFF

[PFF](https://github.com/lovasoa/pff-extract/wiki/Zoomify-PFF-file-format-documentation)
is an old zoomable image file format format developed by zoomify.
You can give a pff meta-information URL (one that contains `requestType=1`)
to dezoomify-rs and it will download it. 

### Krpano

[Krpano](https://krpano.com/home/) is a zoomable image format often used
for panoramas, virtual tours, photoshperes, and other 3d zoomable images.
dezoomify-rs supports downloading individual image planes from such images.
You need to provide the xml meta-information file for the image.

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

If the numbers have leading zeroes in the URL
(such as `image-01-00.jpg` instead of `image-1-0.jpg`),
then you can specify them in the url template as follows:

```
http://example.com/my_image/image-{{X:02}}-{{Y:02}}.jpg
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
    dezoomify-rs [FLAGS] [OPTIONS] [--] [ARGS]

FLAGS:
        --accept-invalid-certs    Whether to accept connecting to insecure HTTPS servers
        --help                    Prints help information
    -l, --largest                 If several zoom levels are available, then select the largest one
    -V, --version                 Prints version information

OPTIONS:
        --connect-timeout <connect-timeout>
            Time after which we should give up when trying to connect to a server [default: 6s]

    -d, --dezoomer <dezoomer>                      Name of the dezoomer to use [default: auto]
    -H, --header <headers>...
            Sets an HTTP header to use on requests. This option can be repeated in order to set multiple headers. You
            can use `-H "Referer: URL"` where URL is the URL of the website's viewer page in order to let the site think
            you come from the legitimate viewer
        --logging <logging>
            Level of logging verbosity. Set it to "debug" to get all logging messages [default: warn]

    -h, --max-height <max-height>
            If several zoom levels are available, then select the one with the largest height that is inferior to max-
            height
        --max-idle-per-host <max-idle-per-host>
            Maximum number of idle connections per host allowed at the same time [default: 32]

    -w, --max-width <max-width>
            If several zoom levels are available, then select the one with the largest width that is inferior to max-
            width
    -n, --parallelism <parallelism>
            Degree of parallelism to use. At most this number of tiles will be downloaded at the same time [default: 16]

    -r, --retries <retries>
            Number of new attempts to make when a tile load fails before giving up. Setting this to 0 is useful to speed
            up the generic dezoomer, which relies on failed tile loads to detect the dimensions of the image. On the
            contrary, if a server is not reliable, set this value to a higher number [default: 1]
        --retry-delay <retry-delay>
            Amount of time to wait before retrying a request that failed [default: 2s]

        --timeout <timeout>
            Maximum time between the beginning of a request and the end of a response before the request should be
            interrupted and considered failed [default: 30s]

ARGS:
    <input-uri>    Input URL or local file name
    <outfile>      File to which the resulting image should be saved
```

## Documentation
  - For documentation specific to this tool, see the [dezoomify-rs wiki](https://github.com/lovasoa/dezoomify-rs/wiki). Do not hesitate to contribute to it by creating new pages or modifying existing ones.
  - For general purpose documentation about zoomable images, the [dezoomify wiki](https://github.com/lovasoa/dezoomify/wiki) may be useful.

## Batch mode

dezoomify-rs does not yet have the ability to download multiple images at once itself.
However, since it is a commandline application. You can use it within a [for loop](https://ss64.com/nt/for.html) in a [batch script](https://en.wikibooks.org/wiki/Windows_Batch_Scripting) in Windows or a [bash script](https://en.wikibooks.org/wiki/Bash_Shell_Scripting) in Linux, MacOS (or windows with [wsl](https://docs.microsoft.com/en-us/windows/wsl/about)).

For instance, in bash, you could create a file called `urls.txt` containing all the urls you want to dezoomify, and then use [xargs](https://en.wikipedia.org/wiki/Xargs) together with dezoomify-rs : 

```sh
xargs -d '\n' -n 1 ./dezoomify-rs < ./urls.txt
```
