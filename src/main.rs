mod cache;
mod fetch;
mod ffmpeg;
mod minimap;
mod optim;
mod options;
mod progress;
#[cfg(test)]
mod tests;

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs};

use gpx::{Gpx, read};

use geo::{Bearing, Distance, Geodesic, Haversine, InterpolatePoint, Point};

use fs_extra::dir::{get_dir_content, get_size};
use futures::{StreamExt, stream};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use cache::Cache;
use fetch::{Fetch, HttpFetcher};
use ffmpeg::*;
use options::CLI_OPTIONS;
use progress::*;

#[derive(Deserialize, Serialize, Debug, Copy, Clone, Default, PartialEq)]
struct GSVPoint {
    lat: f64,
    lng: f64,
}

#[derive(Deserialize, Serialize, Debug, Copy, Clone, Default, PartialEq)]
struct GPXPoint {
    lat: f64,
    lng: f64,
    ele: Option<f64>,
}

struct ReadResult {
    points: Vec<GPXPoint>,
    name: Option<String>,
    size: u64,
}

#[derive(Deserialize, Serialize, Debug, Copy, Clone, Default, PartialEq)]
struct SerializablePointBearing {
    lat: f64,
    lng: f64,
    bearing: f64,
    ele: Option<f64>,
}

#[derive(Deserialize, Debug, Clone)]
struct GSVMetadata {
    /// Only ever surfaces through the `Debug` output when a point's metadata
    /// comes back not-OK, which dead-code analysis cannot see.
    #[serde(default)]
    #[allow(dead_code)]
    date: String,

    #[serde(default)]
    location: GSVPoint,

    #[serde(default)]
    pano_id: String,

    #[serde(default)]
    status: String,
}

#[derive(Debug, Clone, Copy)]
struct PointBearing {
    point: GPXPoint,
    bearing: f64,
}

/// Serialized as camelCase to keep the on-the-wire JSON contract that
/// `--use-metadata` reads back and downstream consumers expect.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct MetadataResult {
    distance: f64,
    frames: usize,
    gps_points: Vec<SerializablePointBearing>,
    original_points: Vec<GPXPoint>,
    average_error: f64,
    name: String,
    file_size_bytes: u64,
}

impl SerializablePointBearing {
    fn from_geo(pb: &PointBearing) -> SerializablePointBearing {
        SerializablePointBearing {
            bearing: pb.bearing,
            lat: pb.point.lat,
            lng: pb.point.lng,
            ele: pb.point.ele,
        }
    }
}

impl GPXPoint {
    fn to_geo_point(self) -> Point<f64> {
        Point::new(self.lng, self.lat)
    }
}

/// The Street View image request for one point.
fn streetview_image_url(point_bearing: &SerializablePointBearing, api_key: &str) -> String {
    format!(
        "https://maps.googleapis.com/maps/api/streetview?size={VIDEO_WIDTH}x{VIDEO_HEIGHT}&location={},{}&fov=100&source=outdoor&heading={}&pitch=0&key={}",
        point_bearing.lat, point_bearing.lng, point_bearing.bearing, api_key
    )
}

/// The Street View metadata request for one point.
fn streetview_metadata_url(point_bearing: &PointBearing, api_key: &str) -> String {
    format!(
        "https://maps.googleapis.com/maps/api/streetview/metadata?location={},{}&source=outdoor&key={}",
        point_bearing.point.lat, point_bearing.point.lng, api_key
    )
}

/// The fetch stages of a render, and everything they need to reach Google.
///
/// These arrive as fields rather than being read from [`CLI_OPTIONS`] so a test
/// can hand over a recording fetcher and observe exactly which requests a render
/// makes.
struct Fetching<'a, F: Fetch> {
    fetcher: &'a F,
    cache: &'a Cache,
    api_key: &'a str,
    concurrency: usize,
}

impl<F: Fetch> Fetching<'_, F> {
    /// For each input point_bearing, request the streetview image from Google's static API.
    /// Save each image as {index}.jpg within out_dir.
    async fn images<P: AsRef<Path>>(
        &self,
        point_bearings: &[SerializablePointBearing],
        out_dir: &P,
    ) {
        let total_requests = point_bearings.len();
        let mut requests_completed = 0;
        let bodies = stream::iter(
            point_bearings
                .iter()
                .map(|pb| streetview_image_url(pb, self.api_key))
                .enumerate(),
        )
        .map(|(index, url)| async move {
            let bytes =
                fetch::fetch_cached(self.fetcher, self.cache, cache::Kind::StreetView, &url).await;
            (index, bytes)
        })
        .buffer_unordered(self.concurrency);

        bodies
            .map(|(index, bytes)| {
                requests_completed += 1;
                progress(&format!(
                    "Progress: {:.1}% ({}/{})",
                    (requests_completed as f64 / total_requests as f64) * 100.0,
                    requests_completed,
                    total_requests
                ));
                (index, bytes)
            })
            .for_each(|(index, bytes)| async move {
                let filename = out_dir.as_ref().join(format!("{}.jpg", index));
                let bytes = bytes.expect("Error in streetview image response");
                tokio::fs::write(filename, bytes).await.unwrap();
            })
            .await;
        // TODO: check that the images are all in fact jpg, and not an error message (which is png)
        // TODO: if we see a png image, then convert it to jpg
    }

    /// For each input point_bearing, request its streetview metadata from Google's static API.
    /// Sends requests in parallel determined by network_concurrency option.
    /// Return array of metadata, one item per input point.
    async fn metadata(&self, point_bearings: &[PointBearing]) -> Vec<GSVMetadata> {
        // use metadata requests to skip errors https://developers.google.com/maps/documentation/streetview/metadata
        // and to correct points lat/lng
        // and to skip images that are a copy of the previous one
        let total_request_count = point_bearings.len();
        let mut requests_completed = 0;
        let bodies = stream::iter(
            point_bearings
                .iter()
                .map(|pb| streetview_metadata_url(pb, self.api_key))
                .enumerate(),
        )
        .map(|(index, url)| async move {
            let bytes =
                fetch::fetch_cached(self.fetcher, self.cache, cache::Kind::Metadata, &url).await;
            (index, bytes.expect("Error in streetview metadata response"))
        })
        .buffer_unordered(self.concurrency);

        let mut indexed_metadata = bodies
            .map(|(index, bytes)| {
                requests_completed += 1;
                // Print progress message with requests completed / total requests as percentage
                let percent = (requests_completed as f64 / total_request_count as f64) * 100.0;
                progress(&format!(
                    "Progress: {:.1}% ({}/{})",
                    percent, requests_completed, total_request_count
                ));
                let parsed = serde_json::from_slice::<GSVMetadata>(&bytes)
                    .expect("Could not parse GSV metadata");
                (index, parsed)
            })
            .collect::<Vec<_>>()
            .await;
        indexed_metadata.sort_unstable_by_key(|&(index, _)| index);
        indexed_metadata
            .into_iter()
            .map(|(_, data)| data)
            .collect::<Vec<_>>()
    }

    /// Fetch the minimap images a render needs.
    ///
    /// Every request here is a Maps Static one, so calling this before any
    /// Street View fetch means a key without that API switched on costs nothing
    /// instead of surfacing after the frames are paid for.
    async fn minimaps(&self, urls: &[String]) -> Result<Vec<Vec<u8>>, String> {
        let mut images = Vec::with_capacity(urls.len());
        let fetched = stream::iter(urls.iter().enumerate())
            .map(|(index, url)| async move {
                (
                    index,
                    fetch::fetch_cached(self.fetcher, self.cache, cache::Kind::Map, url).await,
                )
            })
            .buffer_unordered(self.concurrency)
            .collect::<Vec<_>>()
            .await;
        let mut sorted = fetched;
        sorted.sort_unstable_by_key(|(index, _)| *index);
        for (_, result) in sorted {
            match result {
                Ok(bytes) => images.push(bytes),
                Err(err) => return Err(minimap::map_failure_message(&err)),
            }
        }
        Ok(images)
    }
}

/// The two lines the minimap draws: the GPX track as it was recorded, and where
/// the rendered frames actually ended up after Google snapped each sample to its
/// nearest panorama.
fn route_lines(result: &MetadataResult) -> (Vec<minimap::LatLng>, Vec<minimap::LatLng>) {
    let as_latlng = |lat: f64, lng: f64| minimap::LatLng { lat, lng };
    (
        result
            .original_points
            .iter()
            .map(|p| as_latlng(p.lat, p.lng))
            .collect(),
        result
            .gps_points
            .iter()
            .map(|p| as_latlng(p.lat, p.lng))
            .collect(),
    )
}

/// Fetch everything a render pulls from Google, minimaps first.
///
/// The order is the point. A minimap failure has to happen before the Street
/// View frames are paid for, so this owns the sequence rather than leaving it to
/// whoever calls the two stages.
async fn fetch_render_inputs<F: Fetch, P: AsRef<Path>>(
    fetching: &Fetching<'_, F>,
    plan: Option<&minimap::MinimapPlan>,
    metadata_result: &MetadataResult,
    out_dir: &P,
) -> Result<(), String> {
    if let Some(plan) = plan {
        // One map, purely to prove the key can fetch one, and thrown away. The
        // maps the render actually uses are fetched later by `draw_minimaps`,
        // because stages between here and there rewrite the frame list. The
        // cache makes this free when that later fetch asks for the same image.
        progress_stage("Checking the minimap can be fetched");
        let (track, panorama) = route_lines(metadata_result);
        if let Some(probe) = minimap::probe_url(plan, &track, &panorama, fetching.api_key) {
            fetching.minimaps(std::slice::from_ref(&probe)).await?;
        }
    }

    progress_stage("Fetching images from Streetview");
    fetching.images(&metadata_result.gps_points, out_dir).await;
    Ok(())
}

/// Fetch the minimaps a render needs and draw them, against one snapshot of the
/// frame list.
///
/// Fetching and drawing live in the same function on purpose. When they were
/// separate, the optimizer stage sat between them and rewrote the frames, so
/// follow mode drew a map per *pre-optimizer* point onto a video with fewer
/// frames, and overview mode projected dots with a framing that no longer
/// matched the map that had been fetched.
async fn draw_minimaps<F: Fetch, P: AsRef<Path>>(
    fetching: &Fetching<'_, F>,
    plan: &minimap::MinimapPlan,
    metadata_result: &MetadataResult,
    out_dir: &P,
) -> Result<usize, String> {
    progress_stage("Fetching minimap from Google Maps");
    let (track, panorama) = route_lines(metadata_result);
    let urls = minimap::minimap_urls(plan, &track, &panorama, fetching.api_key);
    let maps = fetching.minimaps(&urls).await?;

    progress_stage("Drawing the minimap");
    minimap::render_frames(plan, &maps, &track, &panorama, out_dir)
}

/// Given list of point_bearings and their metadata (expect arrays of same length),
/// Filter out any points whose metadata is not ok and
/// Group together all points that share the same panorama location.
/// Return point_bearings and metadata by selecting the closest point per panorama id.
fn group_by_location(
    point_bearings: Vec<PointBearing>,
    metadata: Vec<GSVMetadata>,
) -> (Vec<PointBearing>, Vec<f64>) {
    let mut grouped_points = vec![vec![]];
    let mut last_pano = None;
    for (point_bearing, meta) in point_bearings
        .into_iter()
        .zip(metadata)
        .filter(|(_, metadata)| {
            let is_ok = metadata.status == "OK";
            if !is_ok {
                eprintln!("Metadata not ok! {:?}", metadata);
            }
            is_ok
        })
    {
        if let Some(last_pano) = last_pano
            && last_pano != meta.pano_id
        {
            grouped_points.push(vec![]);
        }
        let actual_point = point_bearing.point.to_geo_point();
        let pano_point = Point::new(meta.location.lng, meta.location.lat);
        let err = Geodesic.distance(actual_point, pano_point);
        let groups = grouped_points.len();

        last_pano = Some(meta.pano_id.clone());
        grouped_points[groups - 1].push((point_bearing, meta, err));
    }
    let best_groups = grouped_points
        .into_iter()
        .map(|group| {
            group
                .into_iter()
                .min_by_key(|(_, _, err)| ordered_float::OrderedFloat(*err))
                .expect("Could not group streetview points")
        })
        .collect::<Vec<_>>();
    let errs = best_groups.iter().map(|(_, _, e)| *e).collect::<Vec<_>>();
    let point_bearings = best_groups
        .into_iter()
        .map(|(p, _, _)| p)
        .collect::<Vec<_>>();
    (point_bearings, errs)
}

/// Fill *factor* points between each pair of points in input array.
/// Expect output array to have length of points.len() * factor.
fn interp_points(points: Vec<GPXPoint>, factor: usize) -> Vec<GPXPoint> {
    if factor < 2 {
        points
    } else {
        points
            .iter()
            .zip(points.iter().skip(1))
            .flat_map(move |(p1, p2)| {
                let p1geo = p1.to_geo_point();
                let p2geo = p2.to_geo_point();
                Haversine
                    .points_along_line(
                        p1geo,
                        p2geo,
                        Haversine.distance(p1geo, p2geo) / (factor as f64),
                        /* include ends */ false,
                    )
                    .enumerate()
                    .map(move |(i, p)| GPXPoint {
                        lat: p.y(),
                        lng: p.x(),
                        // Also interp the elevation if given at both endpoints
                        ele: p1.ele.and_then(|e1| {
                            p2.ele.map(|e2| e1 + (e2 - e1) * (i as f64 / factor as f64))
                        }),
                    })
            })
            .collect::<Vec<_>>()
    }
}

/// Compute distance from each point to the next of input.
/// Output has length of points.len() - 1.
fn find_distances(points: &[GPXPoint]) -> Vec<f64> {
    points
        .par_iter()
        .zip(points.par_iter().skip(1))
        .map(|(p1, p2)| get_distance(p1, p2))
        .collect()
}

fn sample_points_by_distance(points: &[GPXPoint], n: usize, distances: &[f64]) -> Vec<GPXPoint> {
    let total_dist: f64 = distances.iter().sum();
    let step = total_dist / (n as f64 - 0.99);
    let mut current = 0.0;
    let mut idx = 0;
    let mut sample = Vec::with_capacity(n);
    while sample.len() < n && idx < points.len() {
        if current >= step * sample.len() as f64 {
            sample.push(points[idx]);
        }
        // Bounds check necessary since the last point doesn't have a distance to the next.
        if idx < distances.len() {
            current += distances[idx];
        }
        idx += 1
    }
    sample
}

fn get_bearing(point1: &GPXPoint, point2: &GPXPoint) -> f64 {
    let p1 = point1.to_geo_point();
    let p2 = point2.to_geo_point();
    Haversine.bearing(p1, p2)
}

fn get_distance(point1: &GPXPoint, point2: &GPXPoint) -> f64 {
    let p1 = point1.to_geo_point();
    let p2 = point2.to_geo_point();
    Geodesic.distance(p1, p2)
}

fn find_bearings(points: &[GPXPoint]) -> Vec<PointBearing> {
    let mut results = points
        .par_iter()
        .zip(points.par_iter().skip(1))
        .map(|(p1, p2)| PointBearing {
            point: *p1,
            bearing: get_bearing(p1, p2),
        })
        .collect::<Vec<_>>();
    // Assume the direction of the second-to-last point continues to the end.
    let last_point = points[points.len() - 1];
    let last_bearing = results[results.len() - 1].bearing;
    results.push(PointBearing {
        point: last_point,
        bearing: last_bearing,
    });
    results
}

fn read_gpx<R: std::io::Read>(reader: R) -> ReadResult {
    let gpx: Gpx = read(reader).expect("Could not read gpx");
    let points = gpx
        .tracks
        .into_iter()
        .flat_map(|t| t.segments.into_iter().map(|s| s.points.into_iter()))
        .flatten()
        .map(|p| GPXPoint {
            lat: p.point().y(),
            lng: p.point().x(),
            ele: p.elevation,
        })
        .collect::<Vec<_>>();
    // Estimate each point is about 32 bytes
    let size = (points.len() * 32) as u64;
    ReadResult {
        points,
        name: gpx.metadata.and_then(|m| m.name),
        size,
    }
}

async fn create_video<F: Fetch>(
    fetching: &Fetching<'_, F>,
    output_dir: PathBuf,
    mut metadata_result: MetadataResult,
) {
    // Remove first offset frames from gps points
    metadata_result
        .gps_points
        .drain(0..CLI_OPTIONS.offset_frames.unwrap_or(0));
    // Remove all frames after max frames from gps points
    metadata_result
        .gps_points
        .truncate(CLI_OPTIONS.max_frames.unwrap_or(metadata_result.frames));
    let plan = minimap::MinimapPlan::resolve(
        CLI_OPTIONS.minimap,
        CLI_OPTIONS.minimap_position,
        CLI_OPTIONS.minimap_size,
        CLI_OPTIONS.minimap_margin,
        CLI_OPTIONS.minimap_zoom,
        VIDEO_WIDTH,
        VIDEO_HEIGHT,
    );
    fetch_render_inputs(fetching, plan.as_ref(), &metadata_result, &output_dir)
        .await
        .unwrap_or_else(|message| panic!("{message}"));
    let dir_size = get_size(&output_dir).unwrap_or(0);
    let dir_files = get_dir_content(&output_dir)
        .map(|d| d.files.len())
        .unwrap_or(0);
    progress(&format!(
        "Fetched {} images, output size: {:.2} MB",
        dir_files,
        (dir_size as f64) / 1000000.0
    ));

    let n_points = if CLI_OPTIONS.optimizer.is_some() {
        progress_stage("Optimizing image sequence (removing inconsistencies)");
        let kept_points = optim::optimize_sequence(&output_dir).await;
        metadata_result.gps_points = kept_points
            .iter()
            .map(|&i| metadata_result.gps_points[i])
            .collect::<Vec<_>>();
        kept_points.len()
    } else {
        metadata_result.gps_points.len()
    };

    if CLI_OPTIONS.print_metadata {
        if CLI_OPTIONS.json {
            println!(
                "{}",
                serde_json::to_string(&metadata_result).expect("Serialization failed")
            );
        } else {
            println!("{:?}", metadata_result);
        }
    }

    let original_timelapse_name = format!(
        "{}-original.mp4",
        CLI_OPTIONS
            .output
            .clone()
            .unwrap_or("streetwarp-lapse".to_string())
    );

    // Drawn here rather than beside the image fetch, because the optimizer above
    // rewrites the frame list and the minimaps have to match what the video will
    // actually show.
    let overlay = match plan.as_ref() {
        None => None,
        Some(plan) => {
            let drawn = draw_minimaps(fetching, plan, &metadata_result, &output_dir)
                .await
                .unwrap_or_else(|message| panic!("{message}"));
            assert_eq!(
                drawn, n_points,
                "drew {drawn} minimaps for a {n_points} frame video"
            );
            let (x, y) = minimap::overlay_offsets(plan, VIDEO_WIDTH, VIDEO_HEIGHT);
            Some(Overlay {
                x,
                y,
                size_px: plan.size_px,
            })
        }
    };

    progress_stage(&format!("Joining {} images into video sequence", n_points));
    create_timelapse(&output_dir, n_points, &original_timelapse_name).await;
    let output_timelapse_name = &CLI_OPTIONS
        .output
        .clone()
        .unwrap_or("streetwarp-lapse.mp4".to_string());

    let motion = match CLI_OPTIONS
        .minterp
        .clone()
        .unwrap_or("good".to_string())
        .as_str()
    {
        "skip" => Motion::Skip,
        "fast" => Motion::Blend,
        _ => Motion::Minterp,
    };

    if motion == Motion::Skip && overlay.is_none() {
        // Nothing to do to the video at all, so move it rather than paying for
        // a re-encode that would only cost quality.
        tokio::fs::rename(&original_timelapse_name, &output_timelapse_name)
            .await
            .expect("Could not rename video files");
    } else {
        progress_stage(match motion {
            Motion::Skip => "Compositing the minimap",
            Motion::Blend => "Blending frames to apply blur",
            Motion::Minterp => "Interpolating motion to apply blur",
        });
        finish_timelapse(
            &output_dir,
            n_points,
            motion,
            overlay,
            &original_timelapse_name,
            output_timelapse_name,
        )
        .await;
    }
    let dir_size = get_size(&output_dir).unwrap_or(0);
    progress(&format!(
        "Created video, total output size: {:.2} MB",
        (dir_size as f64) / 1000000.0
    ));
}

#[tokio::main]
async fn main() {
    LazyLock::force(&CLI_OPTIONS);

    let file = File::open(&CLI_OPTIONS.input_path).unwrap();
    let reader = BufReader::new(file);

    let output_dir = CLI_OPTIONS
        .output_dir
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let start = SystemTime::now();
            let now = start
                .duration_since(UNIX_EPOCH)
                .expect("Time went backwards");
            env::temp_dir().join(format!("streetwarp-tmp-{}", now.as_secs()))
        });
    fs::create_dir_all(&output_dir).expect("Could not open output directory");
    if !CLI_OPTIONS.json {
        println!("output dir is {}", output_dir.to_string_lossy());
    }

    let http = HttpFetcher::new();
    let cache = if CLI_OPTIONS.no_cache {
        Cache::disabled()
    } else {
        Cache::in_platform_cache_dir()
    };
    let fetching = Fetching {
        fetcher: &http,
        cache: &cache,
        api_key: &CLI_OPTIONS.api_key,
        concurrency: CLI_OPTIONS.network_concurrency.unwrap_or(40),
    };

    if CLI_OPTIONS.use_metadata {
        progress_stage("Parsing metadata");
        let metadata_result: MetadataResult =
            serde_json::from_reader(reader).expect("Could not parse submitted metadata result");
        create_video(&fetching, output_dir, metadata_result).await;
        return;
    }

    progress_stage("Parsing GPX data");
    progress("Reading GPX file");
    let read_result = read_gpx(reader);
    let original_points = read_result.points;
    let all_points = original_points.clone();

    progress_stage(&format!(
        "Computing distance statistics ({} points)",
        all_points.len()
    ));
    let distances = find_distances(&all_points);
    let distance = distances.iter().sum::<f64>();
    if !CLI_OPTIONS.json {
        println!("distance is {} with {} points", distance, all_points.len());
    }

    // interpolate extra points to have more closely spaced pictures
    // from my observation it looks like Google can give back up to 300 points per mile
    let expected_frames =
        (CLI_OPTIONS.frames_per_mile.unwrap_or(100.0) * distance / 1600.0) as usize;
    let all_points = interp_points(
        all_points,
        CLI_OPTIONS
            .interp
            .unwrap_or(expected_frames / distances.len() + 1),
    );
    let distances = find_distances(&all_points);

    progress_stage("Finding viewpoints");
    let points = find_bearings(&sample_points_by_distance(
        &all_points,
        expected_frames,
        &distances,
    ));
    progress_stage("Fetching Streetview metadata");
    let metadata = fetching.metadata(&points).await;
    progress_stage(&format!(
        "Found metadata for {} streetview points",
        metadata.len()
    ));
    let (points, errs) = group_by_location(points, metadata);

    if !CLI_OPTIONS.json {
        println!(
            "distance is {} with {} points",
            distances.iter().sum::<f64>(),
            all_points.len()
        );
        println!("filtered to {} points", points.len());
        println!(
            "average error is {} meters",
            errs.iter().sum::<f64>() / errs.len() as f64
        );
    }

    let metadata_result = MetadataResult {
        distance: distances.iter().sum::<f64>(),
        frames: points.len(),
        average_error: errs.iter().sum::<f64>() / errs.len() as f64,
        gps_points: points
            .iter()
            .map(SerializablePointBearing::from_geo)
            .collect::<Vec<_>>(),
        original_points,
        name: read_result.name.unwrap_or("Unnamed GPX File".to_owned()),
        file_size_bytes: read_result.size,
    };
    if CLI_OPTIONS.dry_run {
        if CLI_OPTIONS.json {
            println!(
                "{}",
                serde_json::to_string(&metadata_result).expect("Serialization failed")
            );
        } else {
            println!("{:?}", metadata_result);
        }
        return;
    }
    create_video(&fetching, output_dir, metadata_result).await;
}
