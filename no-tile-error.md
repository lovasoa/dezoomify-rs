---
title: 'What to do when dezoomify-rs says "Could not get any tile for the image"'
permalink: /no-tile-error
---

# What to do when dezoomify-rs says "Could not get any tile for the image"

Sometimes an image download seems to be starting successfully in dezoomify-rs, but it ends prematurely with an error that says "Could not get any tile for the image". Sometimes the error message is preceded by error messages mentioning "network error".

### What does it mean ?

This happens when dezoomify-rs has found the zoomable image protocol according to which the individual image tiles should be requested, it has made the requests, but they all failed.

### What to do ?

#### If the server you are trying to download the image from is slow

When dezoomify-rs tries downloading all the small tiles that compose a large image, it makes a lot of requests to the site on which the image is hosted, very fast. Sometimes the site cannot or does not want to handle all these simultaneous requests and responds with error messages instead of the actual image tiles. 
 - You can make dezoomify-rs request a single image tile at a time with the command-line option `--parallelism 1`
 - `--retries 5` to retry each tile download 5 times before giving up
 - `--retry-delay 5s` to wait 5s when a tile download fails before retrying
 - `--timeout 60s` to not give up tile a tile download before having stayed at least 60s without answer from the web server  


In your terminal, you can run

```commandline
/path/to/dezoomify-rs --parallelism 1 --retries 5 --retry-delay 5s --timeout 60s https://example.com/your-image-url
```

#### If you are using the *Generic Dezoomer*

If you are using the *generic* or the *custom YAML* dezoomer, and you see *404* errors, the problem is probably with your input to dezoomify-rs. 

You should check that the URL pattern you are giving to dezoomify-rs corresponds to actual image URLs that you can load in your web browser, and you can follow the [generic dezoomer tutorial](https://github.com/lovasoa/dezoomify/wiki/Generic-dezoomer-tutorial) or the [custom YAML dezoomer tutorial](https://github.com/lovasoa/dezoomify-rs/wiki/Usage-example-for-the-custom-YAML-dezoomer).


#### If you think this is a bug in dezoomify-rs

You can [report bugs in dezoomify-rs on github](https://github.com/lovasoa/dezoomify-rs/issues)