// SPDX-License-Identifier: GPL-3.0-or-later
use super::{children, flag, number, path_node};
use roxmltree::Node;
use solarcraft_model::horizon::SiteShade;

pub(super) fn read(area: Node<'_, '_>) -> Result<Option<SiteShade>, &'static str> {
    let Some(shade) = path_node(area, &["Verluste", "VerschattungsObjekt"]) else { return Ok(None) };
    if !flag(shade, &["IsHorizon"]) {
        let loss = number::<f64>(shade, &["ProzentualeVerschattung"]).unwrap_or(0.0) / 100.0;
        return if (0.0..=1.0).contains(&loss) {
            Ok((loss > 0.0).then_some(SiteShade::Uniform { loss }))
        } else {
            Err("The saved shading percentage is invalid.")
        };
    }
    let invalid = "The saved horizon needs distinct compass angles and elevations between 0° and 90°.";
    let mut points = children(shade, "HorizontPunkte").map(|point| {
        let az = number::<f64>(point, &["Ausrichtung"]).filter(|v| (0.0..=360.0).contains(v)).ok_or(invalid)?;
        let el = number::<f64>(point, &["Hoehenwinkel"]).filter(|v| (0.0..=90.0).contains(v)).ok_or(invalid)?;
        Ok((az.rem_euclid(360.0), el))
    }).collect::<Result<Vec<_>, &str>>()?;
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    points.dedup();
    if points.len() < 2 || points.windows(2).any(|pair| pair[0].0 == pair[1].0) { return Err(invalid); }

    let first = points[0];
    let last = points[points.len() - 1];
    let elevation_deg = (0..360).map(|degree| {
        let az = f64::from(degree);
        let i = points.partition_point(|p| p.0 <= az);
        let a = if i == 0 { (last.0 - 360.0, last.1) } else { points[i - 1] };
        let b = points.get(i).copied().unwrap_or((first.0 + 360.0, first.1));
        a.1 + (b.1 - a.1) * (az - a.0) / (b.0 - a.0)
    }).collect();
    Ok(Some(SiteShade::Horizon { elevation_deg }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolates_through_compass_north() {
        let xml = r#"<area><Verluste><VerschattungsObjekt><IsHorizon>true</IsHorizon>
            <HorizontPunkte><Ausrichtung>10</Ausrichtung><Hoehenwinkel>20</Hoehenwinkel></HorizontPunkte>
            <HorizontPunkte><Ausrichtung>350</Ausrichtung><Hoehenwinkel>10</Hoehenwinkel></HorizontPunkte>
            </VerschattungsObjekt></Verluste></area>"#;
        let doc = roxmltree::Document::parse(xml).unwrap();
        let Some(SiteShade::Horizon { elevation_deg }) = read(doc.root_element()).unwrap() else { panic!("missing horizon") };
        assert_eq!((elevation_deg[350], elevation_deg[0], elevation_deg[10]), (10.0, 15.0, 20.0));
    }
}
