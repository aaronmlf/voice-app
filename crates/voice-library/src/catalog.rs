use super::Preset;
use anyhow::{Context, Result};

pub(super) fn presets() -> Result<Vec<Preset>> {
    serde_json::from_str(include_str!("../catalog/voices.json"))
        .context("read included voice catalog")
}

/// Default artwork for a catalog voice, decoded only when requested by the GUI.
pub fn catalog_thumbnail(id: &str) -> Option<&'static [u8]> {
    Some(match id {
        "custom.fantasy.gigante-pedra" => include_bytes!("../catalog/thumbnails/gigante-pedra.png"),
        "custom.fantasy.goblin-sucateiro" => {
            include_bytes!("../catalog/thumbnails/goblin-sucateiro.png")
        }
        "custom.fiction.automato-exterminador" => {
            include_bytes!("../catalog/thumbnails/automato-exterminador.png")
        }
        "custom.fiction.espectro" => include_bytes!("../catalog/thumbnails/espectro.png"),
        "custom.fiction.ia-laboratorio" => {
            include_bytes!("../catalog/thumbnails/ia-laboratorio.png")
        }
        "custom.fiction.locutor-assombrado" => {
            include_bytes!("../catalog/thumbnails/locutor-assombrado.png")
        }
        "custom.wh40k.chaos-marine" => include_bytes!("../catalog/thumbnails/chaos-marine.png"),
        "custom.wh40k.comissario-vox" => include_bytes!("../catalog/thumbnails/comissario-vox.png"),
        "custom.wh40k.dreadnought" => include_bytes!("../catalog/thumbnails/dreadnought.png"),
        "custom.wh40k.krieg" => include_bytes!("../catalog/thumbnails/krieg.png"),
        "custom.wh40k.marine-vox-leve" => {
            include_bytes!("../catalog/thumbnails/marine-vox-leve.png")
        }
        "custom.wh40k.necron" => include_bytes!("../catalog/thumbnails/necron.png"),
        "custom.wh40k.ork-boy" => include_bytes!("../catalog/thumbnails/ork-boy.png"),
        "custom.wh40k.servo-cranio" => include_bytes!("../catalog/thumbnails/servo-cranio.png"),
        "custom.wh40k.space-marine" => include_bytes!("../catalog/thumbnails/space-marine.png"),
        "custom.wh40k.tecnosacerdote" => include_bytes!("../catalog/thumbnails/tecnosacerdote.png"),
        "custom.wh40k.terminator" => include_bytes!("../catalog/thumbnails/terminator.png"),
        "custom.wh40k.warboss" => include_bytes!("../catalog/thumbnails/warboss.png"),
        _ => return None,
    })
}
