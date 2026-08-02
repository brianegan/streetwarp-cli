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

### Setting your API key

Pass `--api-key`, or set `GOOGLE_API_KEY` and leave the flag off. Prefer the environment variable: a key on the command line goes into your shell history and is visible in `ps` to anything else running on the machine.

With mise, put it in `mise.local.toml`, which is gitignored:

```toml
[env]
GOOGLE_API_KEY = "your-key-here"
```

`mise trust` once, and the key is set for any command you run in this directory. `--help` shows the variable name but never its value.

### Running without a surprise bill

The API bills per request and nothing here asks before spending, so set your limits at Google's end first. In the Cloud console, restrict the key to just the Street View Static and Maps Static APIs, and set a [quota cap](https://console.cloud.google.com/apis/api/street-view-image-backend.googleapis.com/quotas) and a billing budget alert. That is the only limit that holds if a command goes wrong.

Then work up from cheap:

```bash
# Free: metadata only, no imagery. Prints the frame count to cost against.
streetwarp route.gpx --dry-run --json

# A 30 frame taste of the route, about $0.21.
streetwarp route.gpx --max-frames 30 --minimap overview

# Fewer frames per mile is the other dial. Dry run it to see the count fall
# before you pay for it.
streetwarp route.gpx --frames-per-mile 40 --dry-run --json
```

Multiply the `frames` figure from the dry run by $0.007, and add $0.002 per frame again if you use `--minimap follow`. Start with `--minimap overview`, which costs one map request no matter how long the route is.

Worth knowing: `--dry-run` is free but not zero network, since it still makes metadata requests.

Every response is cached on disk by default, so re-rendering a route you have already fetched costs nothing and you can iterate on the minimap freely once the first render is paid for. The cache lives in your platform cache directory (`~/Library/Caches/streetwarp` on macOS, `~/.cache/streetwarp` on Linux) and is keyed by request rather than by API key, so rotating a key keeps it. Pass `--no-cache` to fetch everything again.

### Spotting where a route goes wrong

Google returns no panorama for a road it has never driven: a farm track, a private lane, or a crossing your routing app believes in that is not there. Those samples are dropped and the video splices straight across, so a kilometre of route can vanish in a single frame.

Nothing in the summary numbers says so. The average snap error only measures points that survived, so a missing kilometre leaves it untouched. A route with a river crossing that does not exist reported 3.5 m against a clean route's 3.1 m.

Every run prints the jumps it found, including `--dry-run`, so you see them before paying for a single frame:

```
warning: the video jumps 5 times where Street View has no coverage.
     1.3s  skips   351 m  https://www.google.com/maps/@54.311443,-2.673245,17z
    20.7s  skips  1062 m  https://www.google.com/maps/@54.290168,-2.585234,17z
    54.2s  skips  1037 m  https://www.google.com/maps/@54.229613,-2.608015,17z
```

The timestamps are where to scrub to in the finished video, and the links are where to look on the map. A jump counts when a step is more than ten times the route's own usual frame spacing, so it adapts to `--frames-per-mile` rather than assuming a distance. Where coverage is merely thin rather than absent, several frames in a row each jump a little, and those are reported together as one stretch to check.

### Minimap
Draw a small map over the video showing where each frame sits on the route, so you can see at a glance whether the render wandered off your track.

```
streetwarp route.gpx --minimap overview
```

It draws two lines: your GPX track in blue, and the path the video actually follows in orange. Those differ wherever Google snapped a sample to a panorama on a different road, which is exactly the case worth catching.

| Option | Values | Default | Meaning |
| --- | --- | --- | --- |
| `--minimap` | `off`, `overview`, `follow` | `off` | `overview` holds the whole route still and moves a dot along it. `follow` stays centred on you and pans. |
| `--minimap-position` | `tl`, `tr`, `bl`, `br` | `br` | Which corner it sits in. |
| `--minimap-size` | 1 to 100 | `40` | Percent of the video's shorter side. Zoom steps in powers of two, so the map only sharpens when its pixels double: 40 and 80 are the sizes that buy a level, and values between them are just a bigger picture of the same map. |
| `--minimap-margin` | pixels | `12` | Gap between the minimap and the frame edge. |
| `--minimap-zoom` | 0 to 21 | `16` | Zoom level, follow mode only. |

### Usage
`cargo run -- --help`

Included in this repo are some gpx files you can use to play around with.

### Demo
I provide this program's functionality as a free service at [streetwarp.com](https://streetwarp.com).
