# dezoomify-rs

This is a prototype for a new version of
[dezoomify](https://github.com/lovasoa/dezoomify)
written in [rust](https://www.rust-lang.org/).

The goal of this project is not to replace the traditional zoomify.
However, it would have the potential of being able to dezoom even 
very large images, that currently cannot be dezoomed inside a browser
because of memory constraints.

The usage of the current version of the tool is very simple.
You first have to generate a `tiles.txt` file in the following format:

```
first_tile_x first_tile_y first_tile_url
second_tile_x second_tile_y second_tile_url
...
```

Then call

```
dezoomify-rs result.jpg < tiles.txt
```

This should create a `result.jpg` file containing the dezoomed image.