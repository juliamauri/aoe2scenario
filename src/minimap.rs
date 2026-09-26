use crate::scenario::ScenarioInfo;
use image::{Rgb, RgbImage, Rgba, RgbaImage};

const TILE_WIDTH: u32 = 8;
const TILE_HEIGHT: u32 = 4;
const PADDING: u32 = 4;

pub(crate) struct MinimapLayers {
    pub(crate) terrain: RgbImage,
    pub(crate) gaia: RgbaImage,
    pub(crate) players: RgbaImage,
}

#[derive(Clone, Copy)]
struct TerrainPalette {
    up: [u8; 3],
    level: [u8; 3],
    down: [u8; 3],
}

const GRASS: TerrainPalette = TerrainPalette {
    up: [0, 169, 0],
    level: [51, 151, 39],
    down: [0, 141, 0],
};

const FOREST: TerrainPalette = TerrainPalette {
    up: [37, 116, 57],
    level: [21, 118, 21],
    down: [0, 114, 0],
};

const DIRT: TerrainPalette = TerrainPalette {
    up: [243, 170, 92],
    level: [228, 162, 82],
    down: [218, 156, 105],
};

const BEACH_DESERT: TerrainPalette = TerrainPalette {
    up: [248, 201, 138],
    level: [232, 180, 120],
    down: [189, 150, 111],
};

const FARM: TerrainPalette = TerrainPalette {
    up: [138, 139, 87],
    level: [130, 136, 77],
    down: [118, 130, 65],
};

const SHALLOWS: TerrainPalette = TerrainPalette {
    up: [84, 146, 176],
    level: [84, 146, 176],
    down: [84, 146, 176],
};

const WATER: TerrainPalette = TerrainPalette {
    up: [48, 93, 182],
    level: [48, 93, 182],
    down: [48, 93, 182],
};

const DEEP_WATER: TerrainPalette = TerrainPalette {
    up: [0, 74, 161],
    level: [0, 74, 161],
    down: [0, 74, 161],
};

const MEDIUM_WATER: TerrainPalette = TerrainPalette {
    up: [0, 74, 187],
    level: [0, 74, 187],
    down: [0, 74, 187],
};

const AZURE_WATER: TerrainPalette = TerrainPalette {
    up: [0, 84, 176],
    level: [0, 84, 176],
    down: [0, 84, 176],
};

const ICE: TerrainPalette = TerrainPalette {
    up: [152, 192, 240],
    level: [152, 192, 240],
    down: [152, 192, 240],
};

const BLACK: TerrainPalette = TerrainPalette {
    up: [28, 28, 28],
    level: [28, 28, 28],
    down: [28, 28, 28],
};

const ROAD_FUNGUS: TerrainPalette = TerrainPalette {
    up: [243, 170, 92],
    level: [228, 162, 82],
    down: [189, 209, 253],
};

fn terrain_palette(terrain_id: u32) -> TerrainPalette {
    match terrain_id {
        0 | 5 | 9 | 12 | 16 | 60 | 68 | 69 | 70 | 71 | 72 | 73 | 74 | 76 | 77 | 83 | 84 | 85
        | 86 | 87 | 100 => GRASS,

        10 | 13 | 17 | 18 | 19 | 20 | 21 | 48 | 49 | 50 | 55 | 56 | 88 | 89 | 91 | 92 | 104
        | 105 | 106 => FOREST,

        3 | 6 | 11 | 24 | 25 | 27 | 36 | 40 | 41 | 42 | 78 => DIRT,

        2 | 14 | 45 | 46 | 51 | 52 | 53 | 79 | 80 | 81 | 82 | 101 | 102 | 107 | 108 | 109 => {
            BEACH_DESERT
        }

        7 | 8 | 29 | 30 | 31 | 32 | 63 | 64 | 65 | 66 | 67 => FARM,

        4 | 54 | 59 | 90 | 93 | 94 => SHALLOWS,

        1 | 15 | 28 | 95 | 96 | 97 | 98 | 99 => WATER,

        22 | 57 => DEEP_WATER,

        23 => MEDIUM_WATER,

        58 => AZURE_WATER,

        26 | 35 | 37 => ICE,

        47 => BLACK,

        75 => ROAD_FUNGUS,

        110 | 112 | 113 | 128 => FOREST,

        111 | 115 => SHALLOWS,

        114 | 130 => WATER,
        116 => DEEP_WATER,

        117..=121 => FARM,

        122 | 123 => GRASS,

        124..=126 => FARM, // temporary snow-family fallback
        127 => ICE,

        129 => BLACK,

        // obsolete terrains
        33 | 34 => FARM, // snow terrains
        38 | 43 | 44 | 103 => DIRT,
        39 => DIRT, // fungus road
        61 | 62 => GRASS,

        _ => TerrainPalette {
            up: [255, 0, 255],
            level: [255, 0, 255],
            down: [255, 0, 255],
        },
    }
}

fn player_color(color_id: i32) -> [u8; 3] {
    match color_id {
        0 => [0, 0, 255],     // Blue
        1 => [255, 0, 0],     // Red
        2 => [0, 200, 0],     // Green
        3 => [255, 255, 0],   // Yellow
        4 => [0, 255, 255],   // Cyan
        5 => [160, 32, 240],  // Purple
        6 => [128, 128, 128], // Gray
        7 => [255, 128, 0],   // Orange
        _ => [255, 0, 255],
    }
}

fn elevation_at(scenario: &ScenarioInfo, x: i32, y: i32) -> Option<u32> {
    if x < 0 || y < 0 || x >= scenario.width as i32 || y >= scenario.height as i32 {
        return None;
    }

    let index = (y as u32 * scenario.width + x as u32) as usize;
    scenario.elevation.get(index).copied()
}

fn elevation_color(scenario: &ScenarioInfo, x: i32, y: i32, palette: TerrainPalette) -> [u8; 3] {
    let current = elevation_at(scenario, x, y).unwrap();

    let upper_left = elevation_at(scenario, x - 1, y).unwrap_or(current);
    let upper_right = elevation_at(scenario, x, y - 1).unwrap_or(current);

    let lower_left = elevation_at(scenario, x, y + 1).unwrap_or(current);
    let lower_right = elevation_at(scenario, x + 1, y).unwrap_or(current);

    let upper = upper_left + upper_right;
    let lower = lower_left + lower_right;

    if upper > lower {
        palette.up
    } else if lower > upper {
        palette.down
    } else {
        palette.level
    }
}

fn map_to_screen(scenario: &ScenarioInfo, x: f64, y: f64) -> (i32, i32) {
    let half_width = TILE_WIDTH as f64 / 2.0;
    let half_height = TILE_HEIGHT as f64 / 2.0;

    let origin_x = scenario.height as f64 * half_width + PADDING as f64;
    let origin_y = PADDING as f64;

    let screen_x = origin_x + (x - y) * half_width;
    let screen_y = origin_y + (x + y) * half_height;

    (screen_x.round() as i32, screen_y.round() as i32)
}

fn minimap_dimensions(scenario: &ScenarioInfo) -> (u32, u32) {
    let width = (scenario.width + scenario.height) * (TILE_WIDTH / 2) + PADDING * 2;
    let height = (scenario.width + scenario.height) * (TILE_HEIGHT / 2) + PADDING * 2;

    (width, height)
}

pub(crate) fn render_terrain_layer(scenario: &ScenarioInfo) -> RgbImage {
    let half_width = TILE_WIDTH as f32 / 2.0;
    let half_height = TILE_HEIGHT as f32 / 2.0;
    let (image_width, image_height) = minimap_dimensions(scenario);

    let mut image = RgbImage::new(image_width, image_height);

    let origin_x = (scenario.height * (TILE_WIDTH / 2) + PADDING) as f32;
    let origin_y = PADDING as f32;

    for screen_y in 0..image_height {
        for screen_x in 0..image_width {
            let projected_x = (screen_x as f32 - origin_x) / half_width;
            let projected_y = (screen_y as f32 - origin_y) / half_height;

            let map_x = (projected_x + projected_y) / 2.0;
            let map_y = (projected_y - projected_x) / 2.0;

            let tile_x = map_x.floor() as i32;
            let tile_y = map_y.floor() as i32;

            if tile_x >= 0
                && tile_y >= 0
                && tile_x < scenario.width as i32
                && tile_y < scenario.height as i32
            {
                let index = (tile_y as u32 * scenario.width + tile_x as u32) as usize;
                let terrain_id = scenario.terrain[index];
                let palette = terrain_palette(terrain_id);
                let color = elevation_color(scenario, tile_x, tile_y, palette);

                image.put_pixel(screen_x, screen_y, Rgb(color));
            }
        }
    }

    image
}

fn render_gaia_layer(scenario: &ScenarioInfo) -> RgbaImage {
    let (image_width, image_height) = minimap_dimensions(scenario);

    let mut image = RgbaImage::new(image_width, image_height);

    for unit in &scenario.units {
        if unit.player == 0 && unit.type_id == 285 {
            let (screen_x, screen_y) = map_to_screen(scenario, unit.x, unit.y);
            for offset_y in -1..=1 {
                for offset_x in -1..=1 {
                    let marker_x = screen_x + offset_x;
                    let marker_y = screen_y + offset_y;

                    if marker_x >= 0
                        && marker_y >= 0
                        && marker_x < image_width as i32
                        && marker_y < image_height as i32
                    {
                        image.put_pixel(
                            marker_x as u32,
                            marker_y as u32,
                            Rgba([255, 255, 255, 255]),
                        );
                    }
                }
            }
        }
    }

    image
}

fn render_player_layer(scenario: &ScenarioInfo) -> RgbaImage {
    let (image_width, image_height) = minimap_dimensions(scenario);

    let mut image = RgbaImage::new(image_width, image_height);

    for unit in &scenario.units {
        if unit.player != 0 {
            let color = scenario
                .players
                .iter()
                .find(|player| player.id == unit.player)
                .map(|player| player_color(player.color))
                .unwrap_or([255, 0, 255]);

            let (screen_x, screen_y) = map_to_screen(scenario, unit.x, unit.y);
            for offset_y in -1..=1 {
                for offset_x in -1..=1 {
                    let marker_x = screen_x + offset_x;
                    let marker_y = screen_y + offset_y;

                    if marker_x >= 0
                        && marker_y >= 0
                        && marker_x < image_width as i32
                        && marker_y < image_height as i32
                    {
                        image.put_pixel(
                            marker_x as u32,
                            marker_y as u32,
                            Rgba([color[0], color[1], color[2], 255]),
                        );
                    }
                }
            }
        }
    }

    image
}

pub(crate) fn render_isometric_minimap_layers(scenario: &ScenarioInfo) -> MinimapLayers {
    MinimapLayers {
        terrain: render_terrain_layer(scenario),
        gaia: render_gaia_layer(scenario),
        players: render_player_layer(scenario),
    }
}
