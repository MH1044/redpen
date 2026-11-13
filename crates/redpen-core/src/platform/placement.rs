//! Where the popup goes (docs/modules/06-popup-and-settings-ui.md): below the caret with an
//! 8 px gap, or at the mouse when the app shows no caret; above instead when there is no room
//! below; always inside the work area (the monitor without its taskbar) that holds the caret or
//! the mouse. A pure function over physical pixels, so it is tested without a screen.

use super::capture::{Point, Rect};

/// The gap between the caret and the popup, in pixels.
pub const CARET_GAP: i32 = 8;

/// The popup's top-left corner for a popup of `size` (width, height).
///
/// - With a caret: its left edge, [`CARET_GAP`] below its bottom; when the popup would end
///   below the work area, its bottom [`CARET_GAP`] above the caret's top instead.
/// - Without one: at `cursor`; when the popup would end below the work area, its bottom at the
///   cursor instead.
/// - Then clamped into the work area holding the caret's top-left corner (or the cursor), or
///   the nearest work area when none holds it. A popup larger than the work area keeps its
///   top-left corner inside it. With no work areas at all, nothing is clamped.
pub fn place(caret: Option<Rect>, cursor: Point, work_areas: &[Rect], size: (u32, u32)) -> Point {
    let width = i32::try_from(size.0).unwrap_or(i32::MAX);
    let height = i32::try_from(size.1).unwrap_or(i32::MAX);
    let (anchor, top, bottom, gap) = match caret {
        Some(c) => (
            Point { x: c.x, y: c.y },
            c.y,
            c.y.saturating_add(c.height.max(0)),
            CARET_GAP,
        ),
        None => (cursor, cursor.y, cursor.y, 0),
    };
    let Some(area) = area_for(anchor, work_areas) else {
        return Point {
            x: anchor.x,
            y: bottom.saturating_add(gap),
        };
    };
    let area_bottom = area.y.saturating_add(area.height);
    let below = bottom.saturating_add(gap);
    let y = if below.saturating_add(height) <= area_bottom {
        below
    } else {
        top.saturating_sub(gap).saturating_sub(height)
    };
    Point {
        x: clamp(anchor.x, area.x, area.x.saturating_add(area.width), width),
        y: clamp(y, area.y, area_bottom, height),
    }
}

/// `start` moved so `[start, start + len)` lies within `[low, high)` where it fits; a span longer
/// than the room starts at `low`.
fn clamp(start: i32, low: i32, high: i32, len: i32) -> i32 {
    let last = high.saturating_sub(len);
    if last < low {
        return low;
    }
    start.clamp(low, last)
}

/// The work area holding `p`, else the one nearest to it (by the squared distance from `p` to
/// the rectangle), else `None` when there is none.
fn area_for(p: Point, work_areas: &[Rect]) -> Option<Rect> {
    work_areas
        .iter()
        .find(|a| contains(a, p))
        .or_else(|| work_areas.iter().min_by_key(|a| distance_squared(a, p)))
        .copied()
}

fn contains(a: &Rect, p: Point) -> bool {
    p.x >= a.x
        && p.y >= a.y
        && i64::from(p.x) < i64::from(a.x) + i64::from(a.width)
        && i64::from(p.y) < i64::from(a.y) + i64::from(a.height)
}

fn distance_squared(a: &Rect, p: Point) -> i64 {
    let axis = |v: i32, start: i32, len: i32| -> i64 {
        let (v, start, end) = (
            i64::from(v),
            i64::from(start),
            i64::from(start) + i64::from(len),
        );
        if v < start {
            start - v
        } else if v >= end {
            v - end + 1
        } else {
            0
        }
    };
    let dx = axis(p.x, a.x, a.width);
    let dy = axis(p.y, a.y, a.height);
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (u32, u32) = (440, 360);

    fn area(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn caret(x: i32, y: i32) -> Option<Rect> {
        Some(Rect {
            x,
            y,
            width: 1,
            height: 16,
        })
    }

    fn one_monitor() -> Vec<Rect> {
        vec![area(0, 0, 1920, 1040)]
    }

    #[test]
    fn below_the_caret_with_a_gap() {
        let p = place(caret(300, 200), Point { x: 5, y: 5 }, &one_monitor(), SIZE);
        assert_eq!(p, Point { x: 300, y: 224 });
    }

    #[test]
    fn above_the_caret_at_the_bottom_edge() {
        let p = place(caret(300, 1020), Point::default(), &one_monitor(), SIZE);
        // Its bottom edge is 8 px above the caret's top.
        assert_eq!(
            p,
            Point {
                x: 300,
                y: 1020 - 8 - 360
            }
        );
    }

    #[test]
    fn at_the_cursor_without_a_caret() {
        let p = place(None, Point { x: 700, y: 400 }, &one_monitor(), SIZE);
        assert_eq!(p, Point { x: 700, y: 400 });
        // Near the bottom, its bottom edge sits at the cursor.
        let p = place(None, Point { x: 700, y: 900 }, &one_monitor(), SIZE);
        assert_eq!(p, Point { x: 700, y: 540 });
    }

    #[test]
    fn clamped_at_the_right_edge() {
        let p = place(caret(1900, 100), Point::default(), &one_monitor(), SIZE);
        assert_eq!(
            p,
            Point {
                x: 1920 - 440,
                y: 124
            }
        );
    }

    #[test]
    fn inside_the_monitor_that_holds_the_caret() {
        let monitors = vec![area(0, 0, 1920, 1040), area(1920, 0, 1920, 1040)];
        let p = place(caret(3800, 50), Point::default(), &monitors, SIZE);
        assert_eq!(
            p,
            Point {
                x: 3840 - 440,
                y: 74
            }
        );
        // A caret at the left edge of the second monitor stays on it.
        let p = place(caret(1925, 500), Point::default(), &monitors, SIZE);
        assert!(p.x >= 1920, "{p:?}");
    }

    #[test]
    fn a_monitor_left_of_the_main_one() {
        let monitors = vec![area(0, 0, 1920, 1040), area(-1280, 0, 1280, 984)];
        let p = place(caret(-100, 970), Point::default(), &monitors, SIZE);
        assert_eq!(
            p,
            Point {
                x: -440,
                y: 970 - 8 - 360
            }
        );
    }

    #[test]
    fn a_point_outside_every_area_uses_the_nearest() {
        let monitors = vec![area(0, 0, 1920, 1040), area(1920, 0, 1920, 1040)];
        let p = place(None, Point { x: 5000, y: 10 }, &monitors, SIZE);
        assert_eq!(
            p,
            Point {
                x: 3840 - 440,
                y: 10
            }
        );
    }

    #[test]
    fn a_popup_larger_than_the_area_starts_at_its_corner() {
        let p = place(
            caret(10, 10),
            Point::default(),
            &[area(0, 0, 300, 200)],
            SIZE,
        );
        assert_eq!(p, Point { x: 0, y: 0 });
    }

    #[test]
    fn no_work_area_means_no_clamp() {
        let p = place(caret(10, 10), Point::default(), &[], SIZE);
        assert_eq!(p, Point { x: 10, y: 34 });
    }
}
