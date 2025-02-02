use conrod_core::{
    widget::{id::List, Image, Line, Text},
    Colorable, Positionable, Sizeable, UiCell, Widget,
};
use glam::DVec2;

use crate::tile::{self, *};

/// Projects a x world location combined with a viewport to determine the x pixel location in the
/// conrad coordinate system
pub fn world_x_to_pixel_x(
    world_x: f64,
    viewport: &crate::map::WorldViewport,
    window_width: f64,
) -> f64 {
    let half_width = window_width / 2.0;
    crate::util::map(
        viewport.top_left.x,
        viewport.bottom_right.x,
        world_x,
        -half_width,
        half_width,
    )
}

/// Projects a y world location combined with a viewport to determine the y pixel location in the
/// conrad coordinate system
pub fn world_y_to_pixel_y(
    world_y: f64,
    viewport: &crate::map::WorldViewport,
    window_height: f64,
) -> f64 {
    let half_height = window_height / 2.0;
    crate::util::map(
        viewport.bottom_right.y,
        viewport.top_left.y,
        world_y,
        -half_height,
        half_height,
    )
}

/// Returns how many degrees should between lines given the viewport range (in world coordinates), and the size
/// of the window, either width or height, depending on which dimension these lines are for
fn line_distance_for_viewport_degrees(world_range: f64, dimension_size: f64) -> f64 {
    // A neive approximation is ok here since we are only determining the distance between lines
    let range_degrees = world_range * 180.0;

    // Range in degrees, adjusted for screen size
    let mapped_range = range_degrees * 500.0 / dimension_size;
    const DISTANCE_SCALE: f64 = 2.0;

    // Define nice distance values between lines for large distances
    let mapping = [45.0, 15.0, 5.0, 2.0, 1.0];
    for distance in mapping {
        let min_range = distance * DISTANCE_SCALE;
        if mapped_range > min_range {
            return distance;
        }
    }

    let power = (mapped_range / DISTANCE_SCALE).log10();
    let part = power.rem_euclid(1.0);
    //We know the scale and where the number falls within the exponential range
    //so use math to find the correct spacing

    let int_power = power.ceil() as i32;

    if part >= 0.5 {
        0.5 * 10.0f64.powi(int_power)
    } else if part >= 0.2 {
        0.2 * 10.0f64.powi(int_power)
    } else {
        0.1 * 10.0f64.powi(int_power)
    }
}

fn world_width_from_longitude(lng: f64) -> f64 {
    // The world is 360 degrees around, and in world coordinates, 1.0 units around
    lng / 360.0
}

/// The state needed to render the map.
///
/// Implemented as a struct to reduce the number of parameters passed to the map_render function
pub struct MapRendererState<'a, 'b, 'c, 'd, 'e> {
    pub tile_cache: &'a mut tile::PipelineMap,
    pub view: &'b crate::map::TileView,
    pub display: &'c glium::Display,
    pub image_map: &'d mut conrod_core::image::Map<glium::Texture2d>,
    pub ids: &'e mut crate::Ids,
    pub weather_enabled: bool,
}

/// Draws the satellite tiles, weather tiles (if enabled), latitude lines, and longitude lines,
/// using the `view` inside `state`
pub fn draw(state: MapRendererState, ui: &mut UiCell<'_>, font: conrod_core::text::font::Id) {
    let _scope = crate::profile_scope("map_renderer::draw");
    //Or value is okay here because `tile_size()` only returns `None` if no tiles are cached, which
    //only happens the first few frames, therefore this value doesn't need to be accurate
    let tile_cache = state.tile_cache;
    let view = state.view;
    let display = state.display;
    let image_map = state.image_map;
    let ids = state.ids;

    let viewport = state.view.get_world_viewport(ui.win_w, ui.win_h);

    let mut cache_it = tile_cache.values_mut();
    let satellite = cache_it.next().unwrap();
    let weather = cache_it.next().unwrap();

    {
        let _p = crate::profile_scope("Satellite Tile Cache Update");
        satellite.update(&viewport, display, image_map);
    }

    {
        let _p = crate::profile_scope("Weather Tile Cache Update");

        if state.weather_enabled {
            weather.update(&viewport, display, image_map);
        }
    }

    render_tile_set(satellite, view, &mut ids.satellite_tiles, ui);
    if state.weather_enabled {
        render_tile_set(weather, view, &mut ids.weather_tiles, ui);
    }

    // Draw the latitude and longitude lines
    draw_lat_long(&viewport, ui, ids, font);
}

/// Renders a tile set from a provided tile pipeline
pub fn render_tile_set(
    pipeline: &mut TilePipeline,
    view: &crate::map::TileView,
    ids: &mut List,
    ui: &mut UiCell<'_>,
) {
    let tile_size = pipeline.tile_size().unwrap();

    let it = view.tile_iter(tile_size, ui.win_w, ui.win_h);
    let mut size = it.tile_size;
    let offset = it.tile_offset;
    let mut zoom_level = it.tile_zoom;
    let half_width = ui.win_w / 2.0;
    let half_height = ui.win_h / 2.0;

    let tiles_vertically = it.tiles_vertically;

    let tiles: Vec<_> = it.collect();
    {
        let mut guard = crate::MAP_PERF_DATA.lock();
        guard.tiles_rendered = tiles.len();
        guard.zoom = zoom_level;
    }

    // The conrod coordinate system places 0, 0 in the center of the window. Up is the positive y
    // axis, and right is the positive x axis.
    // The units are in terms of screen pixels, so on a window with a size of 1000x500 the point
    // (500, 250) would be the top right corner
    let scope_render_tiles = crate::profile_scope("Render Tiles");

    let mut draw_layers = Vec::new();
    let mut missing = RenderLayer::new(size, zoom_level);

    // Iteratre through each initial tile
    for (i, tile) in tiles.iter().enumerate() {
        let tile_x = i / tiles_vertically as usize;
        let tile_y = i % tiles_vertically as usize;

        let x = offset.x + tile_x as f64 * size.x - half_width + size.x / 2.0;
        let y = offset.y - (tile_y as f64 * size.y) + half_height + size.y / 2.0;

        // Set each one as missing so that we can just use the later loop for everything
        missing.tiles.push((x, y, tile.0, tile.1));
    }

    while !missing.tiles.is_empty() && zoom_level > 0 {
        let mut newest_layer = RenderLayer::new(size, zoom_level);
        let mut new_missing = RenderLayer::new(size * 2.0, zoom_level - 1);

        for (x, y, tile_x, tile_y) in missing.tiles {
            let tile_id = TileId::new(tile_x, tile_y, missing.zoom_level);

            if pipeline.get_tile(tile_id).is_some() {
                let data = (x, y, tile_x, tile_y);
                newest_layer.tiles.push(data);
            } else {
                // If the tile isn't present, add the one that should replace it
                let inner_offset_x = tile_x % 2;
                let inner_offset_y = tile_y % 2;
                let tile_x = tile_x / 2;
                let tile_y = tile_y / 2;

                let x = x - inner_offset_x as f64 * size.x + size.x / 2.0;
                let y = y + inner_offset_y as f64 * size.y - size.y / 2.0;

                let data = (x, y, tile_x, tile_y);

                new_missing.tiles.push(data);
            }
        }

        zoom_level -= 1;
        size *= 2.0;

        draw_layers.push(newest_layer);

        missing = new_missing;
    }

    // We now need to account for more tiles than we currently expect to display
    let mut tile_count = 0;

    for draw_layer in draw_layers.iter() {
        tile_count += draw_layer.tiles.len();
    }

    // Now we resize
    ids.resize(tile_count, &mut ui.widget_id_generator());

    // Otherwise this would draw all of the lower-res images on top of the regular res ones instead
    // of behind like we want
    draw_layers.reverse();

    let mut id_counter = 0;

    for draw_layer in draw_layers {
        let size = draw_layer.size;
        let zoom_level = draw_layer.zoom_level;

        for (x, y, tile_x, tile_y) in draw_layer.tiles {
            let tile_id = TileId::new(tile_x, tile_y, zoom_level);

            if let Some(tile) = pipeline.get_tile(tile_id) {
                Image::new(tile)
                    .x_y(x, y)
                    .w_h(size.x, size.y)
                    .set(ids[id_counter], ui);

                id_counter += 1;
            }
        }
    }

    scope_render_tiles.end();
}

struct RenderLayer {
    pub size: DVec2,
    pub zoom_level: u32,
    pub tiles: Vec<(f64, f64, u32, u32)>,
}

impl RenderLayer {
    pub fn new(size: DVec2, zoom_level: u32) -> Self {
        Self {
            size,
            zoom_level,
            tiles: Vec::new(),
        }
    }
}

/// Draws the lines of latitude and longitude onto the map
pub fn draw_lat_long(
    viewport: &crate::map::WorldViewport,
    ui: &mut UiCell<'_>,
    ids: &mut crate::Ids,
    font: conrod_core::text::font::Id,
) {
    let scope_render_latitude = crate::profile_scope("Render Latitude");
    //Lines of latitude
    let lat_line_distance =
        line_distance_for_viewport_degrees(viewport.bottom_right.y - viewport.top_left.y, ui.win_h);

    let lat_top = crate::util::latitude_from_y(viewport.top_left.y.rem_euclid(1.0));
    let lat_bottom = crate::util::latitude_from_y(viewport.bottom_right.y.rem_euclid(1.0));
    let lat_start = crate::util::modulo_ceil(lat_top, lat_line_distance);

    let lat_lines = ((lat_top - lat_bottom) / lat_line_distance + 1.0).ceil() as usize;

    ids.latitude_lines
        .resize(lat_lines, &mut ui.widget_id_generator());
    ids.latitude_text
        .resize(lat_lines, &mut ui.widget_id_generator());

    let log10_line_distance = lat_line_distance.log10();
    let precision = if log10_line_distance < 0.0 {
        (-log10_line_distance.floor()) as usize
    } else {
        0usize
    };

    const LINE_ALPHA: f32 = 0.4;

    //Latitude decreases as world y increases
    for i in 0..lat_lines {
        let lat = lat_start - i as f64 * lat_line_distance;
        let world_y = crate::util::y_from_latitude(lat);
        let y_pixel = world_y_to_pixel_y(world_y, viewport, ui.win_h);

        let half_width = ui.win_w / 2.0;
        Line::new([-half_width, y_pixel], [half_width, y_pixel])
            //Why does this call need to happen?
            .x_y(0.0, 0.0)
            .color(conrod_core::color::BLACK.alpha(LINE_ALPHA))
            .thickness(1.5)
            .set(ids.latitude_lines[i], ui);

        let text = if lat >= 0.0 {
            format!("{:.1$}°N", lat, precision)
        } else {
            format!("{:.1$}°S", -lat, precision)
        };
        Text::new(text.as_str())
            .top_right()
            .y(y_pixel)
            .color(conrod_core::color::WHITE)
            .font_size(12)
            .font_id(font)
            .set(ids.latitude_text[i], ui);
    }
    scope_render_latitude.end();

    let scope_render_longitude = crate::profile_scope("Render Longitude");
    //Lines of longitude
    let lng_line_distance =
        line_distance_for_viewport_degrees(viewport.bottom_right.x - viewport.top_left.x, ui.win_w);

    let line_distance_world = world_width_from_longitude(lng_line_distance);
    let lng_start = crate::util::modulo_ceil(
        crate::util::longitude_from_x(viewport.top_left.x.rem_euclid(1.0)),
        lng_line_distance,
    );
    let x_start = crate::util::modulo_ceil(viewport.top_left.x, line_distance_world);

    let lng_lines = ((viewport.bottom_right.x - viewport.top_left.x) / line_distance_world + 1.0)
        .ceil() as usize;

    ids.longitude_lines
        .resize(lng_lines, &mut ui.widget_id_generator());
    ids.longitude_text
        .resize(lng_lines, &mut ui.widget_id_generator());

    let log10_line_distance = lng_line_distance.log10();
    let precision = if log10_line_distance < 0.0 {
        (-log10_line_distance.floor()) as usize
    } else {
        0usize
    };

    //Longitude increases as world x increases
    for i in 0..lng_lines {
        let lng = lng_start + i as f64 * lng_line_distance;
        let world_x = x_start + i as f64 * line_distance_world;
        let x_pixel = world_x_to_pixel_x(world_x, viewport, ui.win_w);

        let half_height = ui.win_h / 2.0;
        Line::new([x_pixel, -half_height], [x_pixel, half_height])
            .x_y(0.0, 0.0)
            .color(conrod_core::color::BLACK.alpha(LINE_ALPHA))
            .thickness(1.5)
            .set(ids.longitude_lines[i], ui);

        let text = if lng >= 0.0 {
            format!("{:.1$}°E", lng, precision)
        } else {
            format!("{:.1$}°W", -lng, precision)
        };
        Text::new(text.as_str())
            .bottom_right()
            .x(x_pixel)
            .color(conrod_core::color::WHITE)
            .font_size(12)
            .font_id(font)
            .set(ids.longitude_text[i], ui);
    }

    scope_render_longitude.end();
}
