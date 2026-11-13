//! Page boxes (ISO 32000-2 §14.11.2): Acrobat's Set Page Boxes and Crop, execution plan M4.4.
//!
//! The crop, bleed, trim and art boxes default to their "parent" box (crop → media; the others
//! → crop) and are clipped to the media box. Margins are measured inward from the media box in
//! default user space (unrotated), as Acrobat's dialog does.

use pdfcraft_cos::{Document, Object};

use crate::{OrganizeError, check, walk};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageBox {
    Media,
    Crop,
    Bleed,
    Trim,
    Art,
}

impl PageBox {
    pub fn key(self) -> &'static [u8] {
        match self {
            PageBox::Media => b"MediaBox",
            PageBox::Crop => b"CropBox",
            PageBox::Bleed => b"BleedBox",
            PageBox::Trim => b"TrimBox",
            PageBox::Art => b"ArtBox",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        let lower = s.trim().to_ascii_lowercase();
        match lower.strip_suffix("box").unwrap_or(&lower) {
            "media" => Some(Self::Media),
            "crop" => Some(Self::Crop),
            "bleed" => Some(Self::Bleed),
            "trim" => Some(Self::Trim),
            "art" => Some(Self::Art),
            _ => None,
        }
    }
}

/// What to set a box to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoxSpec {
    /// Back to its default (the parent box).
    Remove,
    /// An absolute rectangle in default user space.
    Rect([f64; 4]),
    /// Margins inward from the media box: left, bottom, right, top (points).
    Margins([f64; 4]),
}

/// Parses a 4-number PDF rectangle `[x0, y0, x1, y1]` into `[xmin, ymin, xmax, ymax]` when it has
/// finite coordinates and positive width and height.
fn rect_of(o: Option<&Object>, doc: &Document) -> Option<[f64; 4]> {
    let o = doc.resolve(o?);
    let v: Vec<f64> = o.as_array()?.iter().filter_map(|x| doc.resolve(x).as_f64()).collect();
    if v.len() != 4 || !v.iter().all(|x| x.is_finite()) {
        return None;
    }
    let r = [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])];
    let (w, h) = (r[2] - r[0], r[3] - r[1]);
    (w > 0.0 && h > 0.0 && w.is_finite() && h.is_finite()).then_some(r)
}

/// Returns the coordinate-wise intersection of two normalized `[xmin, ymin, xmax, ymax]` rectangles.
fn intersect(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])]
}

/// Clips rectangle `a` to `b`, returning `None` when their intersection has no positive area.
fn clip(a: [f64; 4], b: [f64; 4]) -> Option<[f64; 4]> {
    let r = intersect(a, b);
    (r[2] > r[0] && r[3] > r[1]).then_some(r)
}

/// The effective boxes of every page: media, crop, bleed, trim, art.
pub fn page_boxes(doc: &Document) -> Result<Vec<[[f64; 4]; 5]>, OrganizeError> {
    Ok(walk(doc)?
        .iter()
        .map(|(page, inherited)| {
            let obj = doc.get(*page);
            let d = obj.as_dict().cloned().unwrap_or_default();
            let own = |k: &[u8]| rect_of(d.get(k).or_else(|| inherited.get(k)), doc);
            let media = own(b"MediaBox").unwrap_or([0.0, 0.0, 612.0, 792.0]);
            let crop = own(b"CropBox").and_then(|c| clip(c, media)).unwrap_or(media);
            let child = |k: &[u8]| rect_of(d.get(k), doc).and_then(|c| clip(c, media)).unwrap_or(crop);
            [media, crop, child(b"BleedBox"), child(b"TrimBox"), child(b"ArtBox")]
        })
        .collect())
}

/// Set a box on `pages` (0-based). Margins apply to each page's own media box.
pub fn set_page_box(doc: &mut Document, pages: &[usize], which: PageBox, spec: BoxSpec) -> Result<(), OrganizeError> {
    let all = walk(doc)?;
    check(pages, all.len())?;
    let boxes = page_boxes(doc)?;
    let bad = |m: &str| OrganizeError::InvalidBox(m.to_string());
    if which == PageBox::Media && spec == BoxSpec::Remove {
        return Err(bad("the media box can't be removed"));
    }
    // Validate every page first so a failure changes nothing.
    let mut plan = Vec::new();
    for &i in pages {
        let media = boxes[i][0];
        let rect = match spec {
            BoxSpec::Remove => None,
            BoxSpec::Rect(r) => {
                if !r.iter().all(|x| x.is_finite()) {
                    return Err(bad("the rectangle is not a number"));
                }
                let r = [r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])];
                Some(if which == PageBox::Media { r } else { intersect(r, media) })
            }
            BoxSpec::Margins(m) => {
                if !m.iter().all(|x| x.is_finite() && *x >= 0.0) {
                    return Err(bad("margins must be zero or more"));
                }
                Some([media[0] + m[0], media[1] + m[1], media[2] - m[2], media[3] - m[3]])
            }
        };
        if let Some(r) = rect {
            let (w, h) = (r[2] - r[0], r[3] - r[1]);
            if !w.is_finite() || !h.is_finite() {
                return Err(bad("the rectangle is not a number"));
            }
            if w < 1.0 || h < 1.0 {
                return Err(bad(&format!("page {} would be smaller than a point", i + 1)));
            }
        }
        plan.push((i, all[i].0, rect, media));
    }
    for &(_, page, rect, _) in &plan {
        doc.update_dict(page, |d| match rect {
            Some(r) => d.set(which.key().to_vec(), Object::Array(r.iter().map(|v| Object::Real(*v)).collect())),
            None => {
                d.remove(which.key());
            }
        })?;
    }
    if spec == BoxSpec::Remove && which == PageBox::Crop {
        // An inherited crop box from an ancestor `/Pages` node would come back after removal:
        // pin the page's media box explicitly only when an ancestor still supplies a valid CropBox.
        let after = walk(doc)?;
        for &(i, page, _, media) in &plan {
            if after.get(i).and_then(|(_, inh)| rect_of(inh.get(b"CropBox"), doc)).is_some() {
                doc.update_dict(page, |d| d.set(b"CropBox".to_vec(), Object::Array(media.iter().map(|v| Object::Real(*v)).collect())))?;
            }
        }
    }
    Ok(())
}

/// The largest page side ISO 32000-2 Annex C recommends (200 inches in default user space).
pub const MAX_PAGE_SIDE: f64 = 14_400.0;

/// How far one Extend Page step grows a page: 10 cm, room for a few lines of notes.
pub const EXTEND_PAGE_STEP: f64 = 10.0 / 2.54 * 72.0;

/// The side of a page, as it is shown (after its `/Rotate`), that [`extend_page`] grows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageSide {
    Bottom,
    Right,
}

impl PageSide {
    pub fn from_name(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "bottom" => Some(Self::Bottom),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

/// Grows `pages` (0-based) by `amount` points on `side` as each page is shown, leaving blank room
/// to write in, as a notebook page grows. The page content and its comments keep their
/// coordinates: only the media box (and a crop box, when the page has one) moves out on that side,
/// so nothing already on the page moves and the change undoes cleanly. The side is the shown one,
/// so a page turned with `/Rotate` grows downwards on screen too (§7.7.3.3: 90 turns clockwise).
/// Refuses, changing nothing, an amount that is not a positive number or a page that would grow
/// past [`MAX_PAGE_SIDE`].
pub fn extend_page(doc: &mut Document, pages: &[usize], side: PageSide, amount: f64) -> Result<(), OrganizeError> {
    let bad = |m: String| OrganizeError::InvalidBox(m);
    if !amount.is_finite() || amount <= 0.0 {
        return Err(bad("the page can only grow by a positive number of points".into()));
    }
    let all = walk(doc)?;
    check(pages, all.len())?;
    // Each page grows once, however often it is listed.
    let mut unique = pages.to_vec();
    unique.sort_unstable();
    unique.dedup();
    let mut plan = Vec::new();
    for i in unique {
        let Some((page, inherited)) = all.get(i) else {
            return Err(OrganizeError::NoSuchPage(i));
        };
        let d = doc.get(*page).as_dict().cloned().unwrap_or_default();
        let own = |k: &[u8]| rect_of(d.get(k).or_else(|| inherited.get(k)), doc);
        let media = own(b"MediaBox").unwrap_or([0.0, 0.0, 612.0, 792.0]);
        let crop = own(b"CropBox").and_then(|c| clip(c, media));
        let rotate = d.get(b"Rotate").or_else(|| inherited.get(b"Rotate")).and_then(|v| doc.resolve(v).as_int()).unwrap_or(0).rem_euclid(360);
        // Which edge of the unrotated page is shown at the bottom or on the right, and whether
        // growing it means a smaller (min) or larger (max) coordinate.
        let edge = match (side, rotate) {
            (PageSide::Bottom, 90) | (PageSide::Right, 0) => 2,    // x max
            (PageSide::Bottom, 180) | (PageSide::Right, 90) => 3,  // y max
            (PageSide::Bottom, 270) | (PageSide::Right, 180) => 0, // x min
            _ => 1,                                                // y min: Bottom at 0, Right at 270
        };
        let grow = |r: [f64; 4]| {
            let mut r = r;
            if edge < 2 {
                r[edge] -= amount;
            } else {
                r[edge] += amount;
            }
            r
        };
        let new_media = grow(media);
        let (w, h) = (new_media[2] - new_media[0], new_media[3] - new_media[1]);
        if !w.is_finite() || !h.is_finite() || w > MAX_PAGE_SIDE || h > MAX_PAGE_SIDE {
            return Err(bad(format!("page {} would be larger than {MAX_PAGE_SIDE} points (200 inches)", i + 1)));
        }
        // The crop box keeps its other three sides; on the grown side it reaches the new edge.
        let new_crop = crop.map(|c| {
            let mut c = c;
            c[edge] = new_media[edge];
            c
        });
        plan.push((*page, new_media, new_crop));
    }
    let rect = |r: [f64; 4]| Object::Array(r.iter().map(|v| Object::Real(*v)).collect());
    for (page, media, crop) in plan {
        doc.update_dict(page, |d| {
            d.set(b"MediaBox".to_vec(), rect(media));
            if let Some(c) = crop {
                d.set(b"CropBox".to_vec(), rect(c));
            }
        })?;
    }
    Ok(())
}
