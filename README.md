# Hyperlapse streetview images along GPX tracks

![](res/example.gif)

### Prerequisites
1. **Install [ffmpeg](https://ffmpeg.org/download.html)** (or build it with h264 encoding)
2. Get a **Google Maps API key** [from here](https://developers.google.com/maps/documentation/streetview/)
3. Activate the Streetview static API from [this page](https://console.cloud.google.com/apis/library/street-view-image-backend.googleapis.com)
4. To use the minimap overlay, also activate the Maps Static API from [this page](https://console.cloud.google.com/apis/library/static-maps-backend.googleapis.com). It is a separate API on the same key, so a working Streetview key will be refused until you switch it on.
5. Record your API key from [this page](https://console.cloud.google.com/apis/credentials) of the console

### API Usage notes
Some back of the napkin estimations:
  - Google gives you $200 of free credit every month
  - The [Streetview static API](https://developers.google.com/maps/documentation/streetview/) costs $0.007 per frame
  - In the densest areas we can download about 200 images per mile
  - This lets you render about **150** miles per month for free

The minimap uses the [Maps Static API](https://developers.google.com/maps/documentation/maps-static/) at $0.002 per request:
  - `--minimap overview` costs one request per render, whatever the route length
  - `--minimap follow` costs one request per frame, so a render costs about 29% more and your free range drops to roughly **116** miles per month

To **avoid hitting your API quota**, pass in the `--dry-run` option!

Every response is cached on disk by default, so re-rendering a route you have already fetched costs nothing. The cache lives in your platform cache directory (`~/Library/Caches/streetwarp` on macOS, `~/.cache/streetwarp` on Linux) and is keyed by request rather than by API key, so rotating a key keeps it. Pass `--no-cache` to fetch everything again.

### Minimap
Draw a small map over the video showing where each frame sits on the route, so you can see at a glance whether the render wandered off your track.

```
streetwarp route.gpx --api-key KEY --minimap overview
```

It draws two lines: your GPX track in blue, and the path the video actually follows in orange. Those differ wherever Google snapped a sample to a panorama on a different road, which is exactly the case worth catching.

| Option | Values | Default | Meaning |
| --- | --- | --- | --- |
| `--minimap` | `off`, `overview`, `follow` | `off` | `overview` holds the whole route still and moves a dot along it. `follow` stays centred on you and pans. |
| `--minimap-position` | `tl`, `tr`, `bl`, `br` | `br` | Which corner it sits in. |
| `--minimap-size` | 1 to 100 | `30` | Percent of the video's shorter side. |
| `--minimap-margin` | pixels | `12` | Gap between the minimap and the frame edge. |
| `--minimap-zoom` | 0 to 21 | `16` | Zoom level, follow mode only. |

### Usage
`cargo run -- --help`

Included in this repo are some gpx files you can use to play around with.

### Demo
I provide this program's functionality as a free service at [streetwarp.com](https://streetwarp.com).
