//! Note pages: a new page ruled with lines or squares to write on, as in a paper notebook.

use pdfcraft_cos::{Dict, Document, ObjRef, Object, Stream};

use crate::{OrganizeError, insert_blank_page};

/// The ruling of a note page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotePaper {
    /// Horizontal lines 8 mm apart.
    Lined,
    /// Squares of 5 mm, as German "kariert" paper.
    Grid,
}

impl NotePaper {
    pub fn from_name(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "lined" => Some(Self::Lined),
            "grid" => Some(Self::Grid),
            _ => None,
        }
    }
}

const MM: f64 = 72.0 / 25.4;
/// Line spacing of lined paper.
pub const LINE_SPACING: f64 = 8.0 * MM;
/// Square size of grid paper.
pub const GRID_SPACING: f64 = 5.0 * MM;
/// Blank border around the ruling.
const MARGIN: f64 = 10.0 * MM;
/// Lined paper leaves room for a heading above the first line.
const LINED_TOP: f64 = 20.0 * MM;

/// The content stream that draws `paper` on a `width × height` page: thin light blue-grey lines,
/// light enough to write over. The number of lines follows from the size, which the caller keeps
/// within the PDF page limit, so it stays in the low thousands.
pub fn note_paper_content(paper: NotePaper, width: f64, height: f64) -> Vec<u8> {
    let mut s = String::from("q 0.4 w 0.75 0.82 0.9 RG\n");
    let (left, right, bottom) = (MARGIN, width - MARGIN, MARGIN);
    match paper {
        NotePaper::Lined => {
            let mut y = height - LINED_TOP;
            while y >= bottom && right > left {
                s.push_str(&format!("{left:.2} {y:.2} m {right:.2} {y:.2} l\n"));
                y -= LINE_SPACING;
            }
        }
        NotePaper::Grid => {
            let top = height - MARGIN;
            if right > left && top > bottom {
                // Whole squares only: the ruling ends on the last line that fits.
                let cols = ((right - left) / GRID_SPACING).floor();
                let rows = ((top - bottom) / GRID_SPACING).floor();
                let (x1, y0) = (left + cols * GRID_SPACING, top - rows * GRID_SPACING);
                let mut x = left;
                while x <= x1 + 0.01 {
                    s.push_str(&format!("{x:.2} {y0:.2} m {x:.2} {top:.2} l\n"));
                    x += GRID_SPACING;
                }
                let mut y = top;
                while y >= y0 - 0.01 {
                    s.push_str(&format!("{left:.2} {y:.2} m {x1:.2} {y:.2} l\n"));
                    y -= GRID_SPACING;
                }
            }
        }
    }
    s.push_str("S Q\n");
    s.into_bytes()
}

/// Insert a `width × height` note page ruled as `paper` at position `at` (0 = before the first
/// page). Refuses, changing nothing, a size outside 1–14,400 points (ISO 32000-2 Annex C).
pub fn insert_note_page(doc: &mut Document, at: usize, width: f64, height: f64, paper: NotePaper) -> Result<ObjRef, OrganizeError> {
    if !(1.0..=crate::MAX_PAGE_SIDE).contains(&width) || !(1.0..=crate::MAX_PAGE_SIDE).contains(&height) {
        return Err(OrganizeError::InvalidBox(format!("a page is 1 to {} points on each side", crate::MAX_PAGE_SIDE)));
    }
    let content = doc.add(Object::Stream(Stream::flate(Dict::new(), &note_paper_content(paper, width, height))));
    let page = insert_blank_page(doc, at, width, height)?;
    doc.update_dict(page, |d| d.set(b"Contents".to_vec(), Object::Ref(content)))?;
    Ok(page)
}
