//! The editable document and its undo history.

use std::collections::{HashSet, VecDeque};

use canvas::{CodelGrid, Color, Error, Point};

/// The number of edits kept for undo, and for redo.
const MAX_HISTORY: usize = 256;

/// The grid before a resize, kept so a shrink can be undone.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Snapshot {
    width: u32,
    height: u32,
    codels: Vec<Color>,
}

/// One reversible change. The payload is the state to restore.
#[derive(Debug, Clone)]
enum Change {
    /// Restore these colours; every point is recorded once, with the colour it
    /// had before the stroke began.
    Paint(Vec<(Point, Color)>),
    Resize(Snapshot),
    CodelSize(u32),
}

#[derive(Debug, Clone)]
struct Stroke {
    before: Vec<(Point, Color)>,
    seen: HashSet<Point>,
}

/// A Piet image as codels, with an undo history.
///
/// The document is the grid: `codel_size` is the pixel scale used on export and
/// does not change the program. Every mutation bumps `revision`, which is what
/// the analysis cache and the debug session key on.
#[derive(Debug, Clone)]
pub struct Document {
    width: u32,
    height: u32,
    codel_size: u32,
    codels: Vec<Color>,
    revision: u64,
    undo: Vec<Change>,
    redo: Vec<Change>,
    stroke: Option<Stroke>,
}

impl Document {
    /// A white document of `width` x `height` codels at codel size one.
    pub fn new(width: u32, height: u32) -> Result<Self, Error> {
        if width == 0 || height == 0 {
            return Err(Error::EmptyImage);
        }
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or(Error::ImageTooLarge)?;
        Ok(Self {
            width,
            height,
            codel_size: 1,
            codels: vec![Color::White; count],
            revision: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            stroke: None,
        })
    }

    /// Takes over a decoded grid. Import carries the detected codel size.
    pub fn from_grid(grid: &CodelGrid) -> Self {
        Self {
            width: grid.width(),
            height: grid.height(),
            codel_size: grid.codel_size(),
            codels: grid.codels().to_vec(),
            revision: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            stroke: None,
        }
    }

    /// The grid to compile, partition, or export.
    pub fn to_grid(&self) -> Result<CodelGrid, Error> {
        CodelGrid::from_colors_with_codel_size(
            self.width,
            self.height,
            self.codel_size,
            self.codels.clone(),
        )
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn codel_size(&self) -> u32 {
        self.codel_size
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn codels(&self) -> &[Color] {
        &self.codels
    }

    pub fn color(&self, point: Point) -> Option<Color> {
        self.index(point)
            .and_then(|index| self.codels.get(index).copied())
    }

    /// Paints one codel.
    ///
    /// Returns `false` when the point is outside the document or already has
    /// the colour. Inside [`Document::begin_stroke`] and
    /// [`Document::end_stroke`] the recorded changes coalesce into one edit.
    pub fn set(&mut self, point: Point, color: Color) -> bool {
        let Some(old) = self.color(point) else {
            return false;
        };
        if old == color {
            return false;
        }
        self.redo.clear();
        match &mut self.stroke {
            Some(stroke) => {
                if stroke.seen.insert(point) {
                    stroke.before.push((point, old));
                }
            }
            None => self.push_change(Change::Paint(vec![(point, old)])),
        }
        self.write(point, color);
        self.revision += 1;
        true
    }

    /// Floods the 4-connected region of the clicked colour with `color`.
    ///
    /// Four-connectivity is the partition's, so the region a fill covers is the
    /// block a click would select.
    pub fn fill(&mut self, point: Point, color: Color) -> bool {
        let Some(target) = self.color(point) else {
            return false;
        };
        if target == color {
            return false;
        }
        let own_edit = self.stroke.is_none();
        if own_edit {
            self.begin_stroke();
        }

        let mut seen = vec![false; self.codels.len()];
        let mut queue = VecDeque::new();
        queue.push_back(point);
        while let Some(point) = queue.pop_front() {
            let Some(index) = self.index(point) else {
                continue;
            };
            let Some(visited) = seen.get_mut(index) else {
                continue;
            };
            if *visited {
                continue;
            }
            *visited = true;
            if self.color(point) != Some(target) {
                continue;
            }
            self.set(point, color);
            for neighbour in neighbours(point) {
                queue.push_back(neighbour);
            }
        }

        if own_edit {
            self.end_stroke();
        }
        true
    }

    /// Resizes the document, anchored at the upper-left. New codels are white.
    ///
    /// The cropped codels are kept so the shrink can be undone.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<bool, Error> {
        if width == 0 || height == 0 {
            return Err(Error::EmptyImage);
        }
        if width == self.width && height == self.height {
            return Ok(false);
        }
        let count = (width as usize)
            .checked_mul(height as usize)
            .ok_or(Error::ImageTooLarge)?;
        let before = self.snapshot();
        let mut codels = vec![Color::White; count];
        let copy_width = width.min(self.width) as usize;
        let copy_height = height.min(self.height) as usize;
        for y in 0..copy_height {
            let source = y * self.width as usize;
            let destination = y * width as usize;
            codels[destination..destination + copy_width]
                .copy_from_slice(&self.codels[source..source + copy_width]);
        }
        self.width = width;
        self.height = height;
        self.codels = codels;
        self.push_change(Change::Resize(before));
        self.revision += 1;
        Ok(true)
    }

    /// Changes the pixel scale used on export. The codels, and therefore the
    /// program, are unchanged.
    pub fn set_codel_size(&mut self, codel_size: u32) -> bool {
        if codel_size == 0 || codel_size == self.codel_size {
            return false;
        }
        let before = self.codel_size;
        self.codel_size = codel_size;
        self.push_change(Change::CodelSize(before));
        self.revision += 1;
        true
    }

    /// Starts coalescing `set` calls into one undo entry.
    pub fn begin_stroke(&mut self) {
        self.stroke = Some(Stroke {
            before: Vec::new(),
            seen: HashSet::new(),
        });
    }

    /// Ends the stroke, recording it as one edit.
    pub fn end_stroke(&mut self) {
        let Some(stroke) = self.stroke.take() else {
            return;
        };
        if stroke.before.is_empty() {
            return;
        }
        self.push_change(Change::Paint(stroke.before));
        self.revision += 1;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        let Some(change) = self.undo.pop() else {
            return false;
        };
        let inverse = self.revert(change);
        push_capped(&mut self.redo, inverse);
        self.revision += 1;
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(change) = self.redo.pop() else {
            return false;
        };
        let inverse = self.revert(change);
        push_capped(&mut self.undo, inverse);
        self.revision += 1;
        true
    }

    fn index(&self, point: Point) -> Option<usize> {
        if point.x >= self.width || point.y >= self.height {
            return None;
        }
        Some((point.y as usize) * (self.width as usize) + point.x as usize)
    }

    fn write(&mut self, point: Point, color: Color) {
        if let Some(index) = self.index(point)
            && let Some(slot) = self.codels.get_mut(index)
        {
            *slot = color;
        }
    }

    fn push_change(&mut self, change: Change) {
        push_capped(&mut self.undo, change);
        self.redo.clear();
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            width: self.width,
            height: self.height,
            codels: self.codels.clone(),
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.width = snapshot.width;
        self.height = snapshot.height;
        self.codels = snapshot.codels;
    }

    /// Applies the reverse of `change` and returns the change that reverses
    /// that, ready for the other stack.
    fn revert(&mut self, change: Change) -> Change {
        match change {
            Change::Paint(changes) => {
                let mut inverse = Vec::with_capacity(changes.len());
                for (point, before) in changes {
                    let now = self.color(point).unwrap_or(Color::White);
                    self.write(point, before);
                    inverse.push((point, now));
                }
                Change::Paint(inverse)
            }
            Change::Resize(snapshot) => {
                let current = self.snapshot();
                self.restore(snapshot);
                Change::Resize(current)
            }
            Change::CodelSize(size) => {
                let current = self.codel_size;
                self.codel_size = size;
                Change::CodelSize(current)
            }
        }
    }
}

fn push_capped(stack: &mut Vec<Change>, change: Change) {
    stack.push(change);
    if stack.len() > MAX_HISTORY {
        stack.remove(0);
    }
}

fn neighbours(point: Point) -> impl Iterator<Item = Point> {
    let mut points = [None; 4];
    points[0] = point.x.checked_sub(1).map(|x| Point::new(x, point.y));
    points[1] = point.x.checked_add(1).map(|x| Point::new(x, point.y));
    points[2] = point.y.checked_sub(1).map(|y| Point::new(point.x, y));
    points[3] = point.y.checked_add(1).map(|y| Point::new(point.x, y));
    points.into_iter().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas::{Hue, Lightness};
    use proptest::prelude::*;

    fn red() -> Color {
        Color::chromatic(Hue::Red, Lightness::Normal)
    }

    fn green() -> Color {
        Color::chromatic(Hue::Green, Lightness::Normal)
    }

    fn picture(document: &Document) -> (u32, u32, u32, Vec<Color>) {
        (
            document.width(),
            document.height(),
            document.codel_size(),
            document.codels().to_vec(),
        )
    }

    #[test]
    fn painting_undoes_and_redoes() {
        let mut document = Document::new(2, 1).unwrap();
        assert!(document.set(Point::new(0, 0), red()));
        assert_eq!(document.color(Point::new(0, 0)), Some(red()));

        assert!(document.undo());
        assert_eq!(document.color(Point::new(0, 0)), Some(Color::White));
        assert!(document.redo());
        assert_eq!(document.color(Point::new(0, 0)), Some(red()));
    }

    #[test]
    fn a_same_colour_or_out_of_bounds_paint_is_not_an_edit() {
        let mut document = Document::new(1, 1).unwrap();
        assert!(!document.set(Point::new(0, 0), Color::White));
        assert!(!document.set(Point::new(1, 0), red()));
        assert!(!document.can_undo());
        assert_eq!(document.revision(), 0);
    }

    #[test]
    fn a_stroke_is_one_undo_entry() {
        let mut document = Document::new(2, 1).unwrap();
        document.begin_stroke();
        document.set(Point::new(0, 0), red());
        document.set(Point::new(1, 0), green());
        document.end_stroke();

        assert!(document.undo());
        assert_eq!(document.color(Point::new(0, 0)), Some(Color::White));
        assert_eq!(document.color(Point::new(1, 0)), Some(Color::White));
        assert!(!document.undo());
    }

    #[test]
    fn a_stroke_restores_the_colour_it_started_with() {
        let mut document = Document::new(1, 1).unwrap();
        document.set(Point::new(0, 0), red());

        document.begin_stroke();
        document.set(Point::new(0, 0), green());
        document.set(Point::new(0, 0), Color::Black);
        document.end_stroke();

        // The stroke is one edit, and it restores the colour from before it.
        assert!(document.undo());
        assert_eq!(document.color(Point::new(0, 0)), Some(red()));
        assert!(document.undo());
        assert_eq!(document.color(Point::new(0, 0)), Some(Color::White));
        assert!(!document.undo());
    }

    #[test]
    fn a_new_edit_clears_the_redo_stack() {
        let mut document = Document::new(2, 1).unwrap();
        document.set(Point::new(0, 0), red());
        assert!(document.undo());
        assert!(document.can_redo());

        assert!(document.set(Point::new(1, 0), green()));
        assert!(!document.can_redo());
    }

    #[test]
    fn fill_replaces_the_4_connected_region_as_one_edit() {
        let grid = CodelGrid::from_colors(3, 1, vec![red(), red(), green()]).unwrap();
        let mut document = Document::from_grid(&grid);

        assert!(document.fill(Point::new(0, 0), Color::Black));
        assert_eq!(document.color(Point::new(1, 0)), Some(Color::Black));
        assert_eq!(document.color(Point::new(2, 0)), Some(green()));

        assert!(document.undo());
        assert_eq!(document.color(Point::new(0, 0)), Some(red()));
        assert_eq!(document.color(Point::new(1, 0)), Some(red()));
        assert!(!document.undo());
    }

    #[test]
    fn resize_keeps_the_overlap_and_undo_restores_the_crop() {
        let grid =
            CodelGrid::from_colors(2, 2, vec![red(), green(), Color::White, Color::Black]).unwrap();
        let mut document = Document::from_grid(&grid);

        assert!(document.resize(1, 1).unwrap());
        assert_eq!(document.width(), 1);
        assert_eq!(document.height(), 1);
        assert_eq!(document.color(Point::new(0, 0)), Some(red()));

        assert!(document.undo());
        assert_eq!(
            picture(&document),
            (2, 2, 1, vec![red(), green(), Color::White, Color::Black])
        );
    }

    #[test]
    fn a_resize_grows_with_white_codels() {
        let mut document = Document::new(1, 1).unwrap();
        document.set(Point::new(0, 0), red());
        assert!(document.resize(2, 1).unwrap());
        assert_eq!(document.color(Point::new(0, 0)), Some(red()));
        assert_eq!(document.color(Point::new(1, 0)), Some(Color::White));
    }

    #[test]
    fn the_codel_size_is_undoable_and_reaches_the_grid() {
        let mut document = Document::new(2, 2).unwrap();
        assert!(document.set_codel_size(4));
        assert_eq!(document.to_grid().unwrap().codel_size(), 4);

        assert!(document.undo());
        assert_eq!(document.codel_size(), 1);
        assert!(document.redo());
        assert_eq!(document.codel_size(), 4);
    }

    #[test]
    fn from_grid_keeps_the_detected_codel_size() {
        let grid = CodelGrid::from_colors_with_codel_size(2, 1, 3, vec![red(), green()]).unwrap();
        let document = Document::from_grid(&grid);
        assert_eq!(document.codel_size(), 3);
        assert_eq!(picture(&document), (2, 1, 3, vec![red(), green()]));
    }

    fn color_from_code(code: u8) -> Color {
        match code % 5 {
            0 => Color::White,
            1 => Color::Black,
            2 => Color::chromatic(Hue::Red, Lightness::Normal),
            3 => Color::chromatic(Hue::Green, Lightness::Dark),
            _ => Color::chromatic(Hue::Blue, Lightness::Light),
        }
    }

    proptest! {
        #[test]
        fn undoing_every_edit_restores_the_picture(
            edits in proptest::collection::vec((0u32..7, 0u32..7, any::<u8>()), 0..50),
        ) {
            let mut document = Document::new(6, 6).unwrap();
            let original = picture(&document);
            for (x, y, code) in edits {
                document.set(Point::new(x, y), color_from_code(code));
            }
            let edited = picture(&document);

            while document.undo() {}
            prop_assert_eq!(picture(&document), original);

            while document.redo() {}
            prop_assert_eq!(picture(&document), edited);
        }
    }
}
