//! Projection maths for the minimap overlay.
//!
//! Google's Static Maps API and its slippy-map tiles share one coordinate
//! system: a Web Mercator world square 256 pixels on a side at zoom 0, doubling
//! with every zoom level. Everything the minimap draws on top of a fetched map
//! has to agree with that projection exactly, or the dot lands off the road.

/// A geographic coordinate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatLng {
    pub lat: f64,
    pub lng: f64,
}

/// Side length of the Web Mercator world square at zoom 0, in pixels.
const WORLD_SIZE: f64 = 256.0;

/// Project a coordinate to its pixel position within the zoom-0 world square.
///
/// Returns x in `0..WORLD_SIZE` running west to east, and y in `0..WORLD_SIZE`
/// running north to south. Latitudes beyond the Mercator limit of ~85.05°
/// project outside that range rather than being clamped.
pub fn world_pixel(p: LatLng) -> (f64, f64) {
    let x = WORLD_SIZE * (p.lng + 180.0) / 360.0;
    let sin_lat = p.lat.to_radians().sin();
    let y = WORLD_SIZE / 2.0
        - 0.5 * ((1.0 + sin_lat) / (1.0 - sin_lat)).ln() * (WORLD_SIZE / 2.0)
            / std::f64::consts::PI;
    (x, y)
}

/// Project a coordinate to its pixel position within a square map image.
///
/// `center` is the coordinate the image is centred on and `zoom` the zoom it
/// was requested at, both of which have to match the fetched image for the
/// result to line up. `size_px` is the image's side length. Coordinates outside
/// the image's footprint project outside `0..size_px`.
pub fn project(p: LatLng, center: LatLng, zoom: u32, size_px: u32) -> (f64, f64) {
    let scale = (1u64 << zoom) as f64;
    let (px, py) = world_pixel(p);
    let (cx, cy) = world_pixel(center);
    let half = size_px as f64 / 2.0;
    (half + (px - cx) * scale, half + (py - cy) * scale)
}

/// The deepest zoom Google's Static Maps API will serve.
const MAX_ZOOM: u32 = 21;

/// A geographic bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub north: f64,
    pub south: f64,
    pub east: f64,
    pub west: f64,
}

/// The deepest zoom at which `bbox` still fits inside a square image of
/// `size_px`.
///
/// A box with no extent in either direction, which is what a single-point route
/// gives, has no zoom that fails to contain it, so it gets [`MAX_ZOOM`].
pub fn fit_zoom(bbox: BBox, size_px: u32) -> u32 {
    let (west_x, north_y) = world_pixel(LatLng {
        lat: bbox.north,
        lng: bbox.west,
    });
    let (east_x, south_y) = world_pixel(LatLng {
        lat: bbox.south,
        lng: bbox.east,
    });
    let extent = (east_x - west_x).abs().max((south_y - north_y).abs());
    if extent <= 0.0 {
        return MAX_ZOOM;
    }
    let zoom = (size_px as f64 / extent).log2().floor();
    (zoom.clamp(0.0, MAX_ZOOM as f64)) as u32
}

impl BBox {
    /// The box containing every point, or `None` for an empty route.
    pub fn around(points: &[LatLng]) -> Option<BBox> {
        let first = points.first()?;
        Some(points.iter().fold(
            BBox {
                north: first.lat,
                south: first.lat,
                east: first.lng,
                west: first.lng,
            },
            |b, p| BBox {
                north: b.north.max(p.lat),
                south: b.south.min(p.lat),
                east: b.east.max(p.lng),
                west: b.west.min(p.lng),
            },
        ))
    }

    /// The coordinate at the middle of the box.
    pub fn center(&self) -> LatLng {
        LatLng {
            lat: (self.north + self.south) / 2.0,
            lng: (self.east + self.west) / 2.0,
        }
    }
}

/// Encode coordinates using Google's polyline algorithm.
///
/// Static Maps takes a route as `path=enc:<polyline>`, which is far more compact
/// than listing coordinates and is what keeps a long route inside the URL length
/// limit.
pub fn encode_polyline(points: &[LatLng]) -> String {
    /// Append one signed value in the format's chunked base-64 representation.
    fn push_value(out: &mut String, value: i64) {
        // Shift the sign into the low bit and invert a negative, which is what
        // keeps a small delta short in either direction. The result is always
        // non-negative, so the chunking below shifts in zeroes.
        let shifted = value << 1;
        let mut remaining = if value < 0 { !shifted } else { shifted } as u64;
        while remaining >= 0x20 {
            out.push((((remaining & 0x1f) | 0x20) as u8 + 63) as char);
            remaining >>= 5;
        }
        out.push((remaining as u8 + 63) as char);
    }

    let mut encoded = String::new();
    let (mut last_lat, mut last_lng) = (0i64, 0i64);
    for point in points {
        let lat = (point.lat * 1e5).round() as i64;
        let lng = (point.lng * 1e5).round() as i64;
        push_value(&mut encoded, lat - last_lat);
        push_value(&mut encoded, lng - last_lng);
        last_lat = lat;
        last_lng = lng;
    }
    encoded
}

/// Percent-encode a string for use in a URL query value.
///
/// The polyline alphabet runs from `?` to `~`, which takes in `\`, a backtick,
/// `|` and others that are not legal in a URL. Sending them raw gets the path
/// silently dropped or the request rejected, so everything outside the
/// unreserved set is escaped.
pub fn percent_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Google's documented ceiling on a Maps Static API URL is 16384 characters.
/// Staying well inside it leaves room for a longer key or an extra parameter
/// without a route suddenly failing to render.
pub const MAX_URL_LEN: usize = 8192;

/// The GPX track the rider planned.
const TRACK_COLOR: &str = "0x1e88e5ff";
/// The panorama path the video actually follows.
const PANORAMA_COLOR: &str = "0xff6d00ff";

/// Everything needed to ask Google for one minimap image.
pub struct MapRequest<'a> {
    pub center: LatLng,
    pub zoom: u32,
    pub size_px: u32,
    /// The route as recorded in the GPX file.
    pub track: &'a [LatLng],
    /// Where the rendered frames actually sit, after Google snapped each sample
    /// to its nearest panorama.
    pub panorama: &'a [LatLng],
    pub api_key: &'a str,
}

/// Reduce `points` to at most `max`, keeping the first and last.
///
/// A route drawn at minimap size has far more points than it has pixels, so
/// dropping most of them costs nothing visible and is what keeps a long route
/// inside the URL limit.
pub fn downsample(points: &[LatLng], max: usize) -> Vec<LatLng> {
    if points.len() <= max || max == 0 {
        return points.to_vec();
    }
    if max == 1 {
        return vec![points[0]];
    }
    // Spread the kept points evenly across the route and pin the last one, so
    // the drawn line still starts and ends where the route does.
    let step = (points.len() - 1) as f64 / (max - 1) as f64;
    (0..max)
        .map(|i| points[((i as f64) * step).round() as usize])
        .collect()
}

/// Keep the points that fall inside the map's footprint, along with the
/// neighbour on each side of every run, so a line crossing the frame still
/// reaches its edges instead of stopping short.
pub fn clip_to_view(points: &[LatLng], center: LatLng, zoom: u32, size_px: u32) -> Vec<LatLng> {
    let on_screen = |p: &LatLng| {
        let (x, y) = project(*p, center, zoom, size_px);
        let limit = size_px as f64;
        (0.0..=limit).contains(&x) && (0.0..=limit).contains(&y)
    };
    points
        .iter()
        .enumerate()
        .filter(|&(i, p)| {
            on_screen(p)
                || i.checked_sub(1).is_some_and(|before| on_screen(&points[before]))
                || points.get(i + 1).is_some_and(on_screen)
        })
        .map(|(_, p)| *p)
        .collect()
}

/// Build the Static Maps request for one minimap image.
///
/// Both routes ride in the same request, downsampled as far as needed to stay
/// under [`MAX_URL_LEN`].
pub fn build_map_url(request: &MapRequest) -> String {
    let render = |budget: usize| {
        let path = |points: &[LatLng], color: &str, weight: u32| {
            let encoded = encode_polyline(&downsample(points, budget));
            format!(
                "&path={}",
                percent_encode(&format!("color:{color}|weight:{weight}|enc:{encoded}"))
            )
        };
        let mut url = format!(
            "https://maps.googleapis.com/maps/api/staticmap?center={},{}&zoom={}&size={}x{}&scale={MAP_SCALE}&maptype=roadmap",
            request.center.lat,
            request.center.lng,
            request.zoom,
            request.size_px,
            request.size_px
        );
        if !request.track.is_empty() {
            url.push_str(&path(request.track, TRACK_COLOR, 2));
        }
        if !request.panorama.is_empty() {
            url.push_str(&path(request.panorama, PANORAMA_COLOR, 3));
        }
        url.push_str(&format!("&key={}", request.api_key));
        url
    };

    let mut budget = request.track.len().max(request.panorama.len()).max(1);
    loop {
        let url = render(budget);
        // Two points is the shortest thing still worth calling a line, so stop
        // there rather than looping forever on an impossible budget.
        if url.len() <= MAX_URL_LEN || budget <= 2 {
            return url;
        }
        budget /= 2;
    }
}

/// Where a user switches the Maps Static API on.
///
/// A Street View key does not carry it, so this is far and away the likeliest
/// reason a minimap request comes back refused, and naming it turns an opaque
/// 403 into something actionable.
pub const MAPS_STATIC_ENABLE_URL: &str =
    "https://console.cloud.google.com/apis/library/static-maps-backend.googleapis.com";

/// Explain a failed minimap request in terms of the thing most likely wrong.
pub fn map_failure_message(error: &crate::fetch::FetchError) -> String {
    let mut message = format!("Minimap request failed: {}", error.message);
    if matches!(error.status, Some(401 | 403)) {
        message.push_str(
            "\nThis usually means the Maps Static API is not enabled for your key. \
             It is a separate API from Street View, so a working Street View key \
             does not carry it.",
        );
    }
    // The request URL is deliberately never quoted here: it carries the API key.
    message.push_str(&format!("\nEnable the Maps Static API at {MAPS_STATIC_ENABLE_URL}"));
    message
}

/// The modes a plan can actually be in.
///
/// `MinimapMode::Off` produces no plan at all, so it has no representation here
/// and the arms that could never run do not have to be written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Overview,
    Follow,
}

/// The resolved minimap settings for one render.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinimapPlan {
    pub mode: Mode,
    pub position: crate::options::MinimapPosition,
    /// Side length of the minimap in video pixels.
    pub size_px: u32,
    pub margin_px: u32,
    /// Only used in follow mode; overview picks its own zoom to fit the route.
    pub zoom: u32,
}

impl MinimapPlan {
    /// Resolve the command line options against the video's dimensions, or
    /// `None` when the minimap is switched off.
    pub fn resolve(
        mode: crate::options::MinimapMode,
        position: crate::options::MinimapPosition,
        size_percent: u32,
        margin_px: u32,
        zoom: u32,
        video_width: u32,
        video_height: u32,
    ) -> Option<MinimapPlan> {
        let mode = match mode {
            crate::options::MinimapMode::Off => return None,
            crate::options::MinimapMode::Overview => Mode::Overview,
            crate::options::MinimapMode::Follow => Mode::Follow,
        };
        // Sizing against the shorter side keeps the minimap square and stops it
        // eating the frame when the video is wide.
        let shorter = video_width.min(video_height);
        Some(MinimapPlan {
            mode,
            position,
            size_px: (shorter * size_percent / 100).max(1),
            margin_px,
            zoom,
        })
    }
}

/// Colour of the position dot and of the ring around it. The ring is what keeps
/// the dot readable over both pale roads and dark parkland.
const DOT_FILL: image::Rgba<u8> = image::Rgba([255, 255, 255, 255]);
const DOT_RING: image::Rgba<u8> = image::Rgba([17, 17, 17, 255]);

/// Draw the position dot at `(x, y)` on a copy of `map`.
///
/// Coordinates outside the image are drawn as far as they reach, so a dot on the
/// very edge of the frame still shows a sliver rather than vanishing.
pub fn stamp_dot(map: &image::RgbaImage, x: f64, y: f64) -> image::RgbaImage {
    let mut stamped = map.clone();
    let (width, height) = stamped.dimensions();
    // Scale the dot with the map so it stays the same visual size whatever the
    // minimap is sized to, but never shrink it below something you can see.
    let radius = (width.min(height) as f64 / 22.0).max(3.0);
    let ring = radius + (radius / 2.5).max(1.0);

    let left = (x - ring).floor().max(0.0) as u32;
    let top = (y - ring).floor().max(0.0) as u32;
    let right = ((x + ring).ceil().max(0.0) as u32).min(width.saturating_sub(1));
    let bottom = ((y + ring).ceil().max(0.0) as u32).min(height.saturating_sub(1));

    for py in top..=bottom {
        for px in left..=right {
            let distance = ((px as f64 - x).powi(2) + (py as f64 - y).powi(2)).sqrt();
            if distance <= radius {
                stamped.put_pixel(px, py, DOT_FILL);
            } else if distance <= ring {
                stamped.put_pixel(px, py, DOT_RING);
            }
        }
    }
    stamped
}

/// Decode a fetched map image.
pub fn decode_map(bytes: &[u8]) -> Result<image::RgbaImage, String> {
    image::load_from_memory(bytes)
        .map(|image| image.to_rgba8())
        .map_err(|e| format!("Could not decode the minimap Google returned: {e}"))
}

/// Google's `scale=2` returns twice as many pixels for the *same* coverage, so
/// one logical map pixel is two pixels in the image that comes back. Framing and
/// projection work in logical pixels; only the final stamp is in image pixels.
pub const MAP_SCALE: u32 = 2;

/// The centre and zoom the overview map is framed at, or `None` for an empty
/// route.
///
/// The request that asks Google for the map and the projection that stamps the
/// dot onto it both read the framing from here. When each worked it out for
/// itself they could disagree, and the dot would sit off the route.
pub fn overview_framing(
    plan: &MinimapPlan,
    track: &[LatLng],
    panorama: &[LatLng],
) -> Option<(LatLng, u32)> {
    let everything = track.iter().chain(panorama).copied().collect::<Vec<_>>();
    let bounds = BBox::around(&everything)?;
    // Fitted against the size the URL asks for, not the size of the image that
    // comes back. `scale=2` doubles the pixels and leaves the coverage alone, so
    // fitting against the doubled figure would choose a zoom one level too deep
    // and run the route off the edges of the map.
    Some((bounds.center(), fit_zoom(bounds, plan.size_px)))
}

/// Where a coordinate lands within the fetched map image, in image pixels.
pub fn image_pixel(p: LatLng, center: LatLng, zoom: u32, logical_size_px: u32) -> (f64, f64) {
    let (x, y) = project(p, center, zoom, logical_size_px);
    (x * MAP_SCALE as f64, y * MAP_SCALE as f64)
}

/// The Static Maps requests a render needs: one shared image in overview mode,
/// one per frame in follow mode.
pub fn minimap_urls(
    plan: &MinimapPlan,
    track: &[LatLng],
    panorama: &[LatLng],
    api_key: &str,
) -> Vec<String> {
    match plan.mode {
        Mode::Overview => {
            let Some((center, zoom)) = overview_framing(plan, track, panorama) else {
                return Vec::new();
            };
            vec![build_map_url(&MapRequest {
                center,
                zoom,
                size_px: plan.size_px,
                track,
                panorama,
                api_key,
            })]
        }
        Mode::Follow => panorama
            .iter()
            .map(|here| {
                build_map_url(&MapRequest {
                    center: *here,
                    zoom: plan.zoom,
                    size_px: plan.size_px,
                    // At follow zoom the whole route is mostly off screen, so
                    // send only what the window can show. Downsampling the full
                    // route instead would draw a coarse zigzag through it.
                    track: &clip_to_view(track, *here, plan.zoom, plan.size_px),
                    panorama: &clip_to_view(panorama, *here, plan.zoom, plan.size_px),
                    api_key,
                })
            })
            .collect(),
    }
}

/// Pixel offsets of the minimap's top-left corner within the video frame.
pub fn overlay_offsets(plan: &MinimapPlan, video_width: u32, video_height: u32) -> (u32, u32) {
    use crate::options::MinimapPosition::*;
    // A minimap sized close to the frame leaves no room for its margin. Each
    // axis is clamped against its own extent, because a map that fits across a
    // 640px width can still hang off a 480px height.
    let far = |extent: u32| extent.saturating_sub(plan.size_px + plan.margin_px);
    let near = |extent: u32| plan.margin_px.min(far(extent));
    match plan.position {
        Tl => (near(video_width), near(video_height)),
        Tr => (far(video_width), near(video_height)),
        Bl => (near(video_width), far(video_height)),
        Br => (far(video_width), far(video_height)),
    }
}

/// Suffix shared by every minimap frame.
///
/// It sits beside the Street View frame it belongs to, under a different suffix
/// so ffmpeg's `%d.jpg` sequence pattern never picks it up.
const FRAME_SUFFIX: &str = "map.png";

/// Filename of the minimap image for frame `index`.
pub fn frame_filename(index: usize) -> String {
    format!("{index}.{FRAME_SUFFIX}")
}

/// The ffmpeg sequence pattern that reads those frames back.
///
/// Derived from the same suffix as [`frame_filename`], because a rename that
/// updated only one of the two would leave ffmpeg looking for files nothing
/// writes, and fail at render time rather than compile time.
pub fn frame_pattern() -> String {
    format!("%d.{FRAME_SUFFIX}")
}

/// Write one minimap image per video frame into `out_dir`.
///
/// Overview mode stamps every frame's dot onto copies of the one fetched map.
/// Follow mode has a map per frame already centred on the rider, so the dot goes
/// in the middle of each.
pub fn render_frames<P: AsRef<std::path::Path>>(
    plan: &MinimapPlan,
    maps: &[Vec<u8>],
    track: &[LatLng],
    panorama: &[LatLng],
    out_dir: &P,
) -> Result<usize, String> {
    let write = |image: &image::RgbaImage, index: usize| {
        image
            .save(out_dir.as_ref().join(frame_filename(index)))
            .map_err(|e| format!("Could not write minimap frame {index}: {e}"))
    };

    match plan.mode {
        Mode::Overview => {
            let base = decode_map(maps.first().ok_or("No minimap was fetched")?)?;
            let (center, zoom) =
                overview_framing(plan, track, panorama).ok_or("The route has no points")?;
            for (index, here) in panorama.iter().enumerate() {
                let (x, y) = image_pixel(*here, center, zoom, plan.size_px);
                write(&stamp_dot(&base, x, y), index)?;
            }
            Ok(panorama.len())
        }
        Mode::Follow => {
            // Follow mode fetches a map centred on each frame, so the two lists
            // are the same list. If they have drifted apart, some stage rewrote
            // the frames between the fetch and here, and every map after the
            // first change is centred on the wrong place. Say so rather than
            // rendering a video that silently points somewhere else.
            if maps.len() != panorama.len() {
                return Err(format!(
                    "Have {} minimaps for {} video frames; they were fetched for a different \
                     set of frames than the video is being drawn from",
                    maps.len(),
                    panorama.len()
                ));
            }
            for (index, bytes) in maps.iter().enumerate() {
                let map = decode_map(bytes)?;
                // Follow mode centres each map on the rider, so the dot is
                // always the middle of its own image.
                let middle = map.width() as f64 / 2.0;
                write(&stamp_dot(&map, middle, map.height() as f64 / 2.0), index)?;
            }
            Ok(maps.len())
        }
    }
}
