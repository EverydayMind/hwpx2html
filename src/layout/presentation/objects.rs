//! Object geometry shared by both writers; no HTML is generated to find
//! a text box's position or its tables.
use super::gradient::{
    band_angle_supported, band_extent, gradient_band_colors, gradient_band_edges,
};
use super::Frame;
use crate::model::{HwpUnit, PositionedObject, ShapeStyle, Table};

/// How a picture's image sits in its display box. Which of the three a
/// picture takes depends on its placement as well as its clip, so the writers
/// do not read a missing frame themselves: the same absence is a whole-box
/// image inline and a natural-size one on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureFit {
    /// The image takes the whole display box (`width:100%;height:100%`) and
    /// the box does not clip it (inline).
    FillBox,
    /// The image keeps its natural size and the box clips it (floating).
    Natural,
    /// The image is scaled and shifted by the frame and the box clips it.
    Frame(Frame),
}

/// What a [`PictureFit`] was worked out from. It is for reports and tests and
/// changes no output: a picture without a usable canvas is told apart from
/// one that selects all of its canvas, though both take the unframed fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureBasis {
    /// The frame maps `imgClip` against `imgDim`.
    ImgDim,
    /// A floating picture whose `imgDim` is unusable maps `imgClip` against
    /// `orgSz`, as before.
    OrgSz,
    /// The picture has no `imgClip`.
    NoClip,
    /// There is a clip, but no canvas to read it against: `imgDim` is zero
    /// (inline), or `orgSz` is as well (floating). The 21 pictures of the
    /// 루이지애나 보도자료 (the same three in seven documents) are these:
    /// their `imgDim` is written as 0x0.
    NoCanvas,
    /// The clip has no width or height.
    EmptyClip,
    /// An inline clip that selects no strict part of the canvas: it covers all
    /// of it or more, or starts before it.
    WholeCanvas,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PicturePlan {
    pub fit: PictureFit,
    pub basis: PictureBasis,
}

/// A picture's image within its display box.
///
/// `imgClip` is in `imgDim` coordinates whatever the placement, so the clip
/// maps onto the display box as `box * imgDim / clip`; `orgSz` is the shape's
/// own size and says nothing about that canvas. A picture that selects all of
/// `imgDim` therefore fills its box, floating or not (a group child's
/// 123960x164340 canvas under an 18543x24583 `orgSz`, 업무보고 section0).
/// Only a floating picture without a usable `imgDim` still reads its clip
/// against `orgSz`, as before; no sample has one.
///
/// Where no frame can be worked out the picture is not given one: it fills
/// the box (inline) or keeps its natural size (floating). A missing canvas is
/// not repaired from `orgSz`, the clip's far corner or the pixel count, none
/// of which is the canvas `imgClip` is written in.
pub fn picture_plan(object: &PositionedObject, inline: bool) -> PicturePlan {
    let unframed = |basis| PicturePlan {
        fit: if inline {
            PictureFit::FillBox
        } else {
            PictureFit::Natural
        },
        basis,
    };
    let Some(crop) = object.crop else {
        return unframed(PictureBasis::NoClip);
    };
    let (source, basis) = if inline || (object.img_dim.width > 0 && object.img_dim.height > 0) {
        (object.img_dim, PictureBasis::ImgDim)
    } else {
        (object.original_size, PictureBasis::OrgSz)
    };
    if source.width <= 0 || source.height <= 0 {
        return unframed(PictureBasis::NoCanvas);
    }
    if crop.width <= 0 || crop.height <= 0 {
        return unframed(PictureBasis::EmptyClip);
    }
    if inline
        && !(crop.x > 0 || crop.y > 0 || crop.width < source.width || crop.height < source.height)
    {
        return unframed(PictureBasis::WholeCanvas);
    }
    PicturePlan {
        fit: PictureFit::Frame(Frame {
            left: -(object.box_units.width.saturating_mul(crop.x) / crop.width),
            top: -(object.box_units.height.saturating_mul(crop.y) / crop.height),
            width: object.box_units.width.saturating_mul(source.width) / crop.width,
            height: object.box_units.height.saturating_mul(source.height) / crop.height,
        }),
        basis,
    }
}

pub fn gradient_box(object: &PositionedObject) -> bool {
    object.kind == "rect"
        && object.children.is_empty()
        && object.shape.as_ref().is_some_and(|shape| {
            band_angle_supported(shape.gradient_angle)
                && shape.gradient.len() == 2
                && shape.gradient_step > 0
        })
}

/// A gradient box the renderer paints as bands: its colours are readable and
/// the bands cover its height. A box that fails this takes the plain layout,
/// whatever its gradient says.
pub fn gradient_drawn(object: &PositionedObject) -> bool {
    let Some(shape) = object.shape.as_ref().filter(|_| gradient_box(object)) else {
        return false;
    };
    let bands = gradient_band_colors(shape.gradient_step, &shape.gradient[0], &shape.gradient[1]);
    let edges = gradient_band_edges(
        band_extent(
            shape.gradient_angle,
            object.box_units.width.max(1),
            object.box_units.height.max(1),
        ),
        shape.gradient_step,
    );
    !bands.is_empty() && edges.len() == bands.len() + 1
}

/// Shared-SVG eligibility is separate from semantic reading order, which
/// applies to other containers too.
pub fn shared_fill_group(object: &PositionedObject) -> bool {
    object.kind == "container"
        && object.shape.is_none()
        && !object.children.is_empty()
        && object.children.iter().all(|child| {
            child.shape.as_ref().is_some_and(|shape| {
                let fillable = (shape.gradient.len() == 2
                    && band_angle_supported(shape.gradient_angle)
                    && shape.gradient_step > 0)
                    || (shape.fill.is_some() && shape.gradient.is_empty());
                child.children.is_empty()
                    && ((matches!(child.kind.as_str(), "rect" | "polygon")
                        && (fillable || !shape.paragraphs.is_empty()))
                        || child.kind == "line")
            })
        })
}

/// Keep ancestor lengths separate: the browser rounds before adding them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextBox {
    pub left: HwpUnit,
    pub top: HwpUnit,
    pub align_top: HwpUnit,
}

impl TextBox {
    pub fn table_offset(self) -> (HwpUnit, HwpUnit) {
        (self.left, self.top + self.align_top)
    }
}

pub fn text_box(shape: &ShapeStyle, height: HwpUnit, pen: HwpUnit) -> TextBox {
    let inner_height = height
        .saturating_sub(shape.margins[2])
        .saturating_sub(shape.margins[3]);
    let spare = inner_height
        .saturating_sub(text_box_content_height(shape))
        .max(0);
    TextBox {
        left: shape.margins[0] + pen / 2,
        top: shape.margins[2] + pen / 2,
        align_top: match shape.vertical_align.as_str() {
            "CENTER" => spare / 2,
            "BOTTOM" => spare,
            _ => 0,
        },
    }
}

/// Tables including their margins take part in alignment, as do lines
/// (성과보고서 별첨4's title table).
pub fn text_box_content_height(shape: &ShapeStyle) -> HwpUnit {
    let lines = shape
        .paragraphs
        .iter()
        .flat_map(|p| &p.lines)
        .map(|line| line.top.saturating_add(line.height.max(line.text_height)));
    let tables = shape.tables.iter().map(|table| {
        let placed = crate::layout::layout_text_box_table(table);
        placed.box_units.y
            + placed.box_units.height
            + table.out_margin_top
            + table.out_margin_bottom
    });
    lines.chain(tables).max().unwrap_or(0)
}

pub fn text_box_tables(shape: &ShapeStyle, offset: (HwpUnit, HwpUnit)) -> Vec<Table> {
    shape
        .tables
        .iter()
        .map(|table| {
            let mut placed = crate::layout::layout_text_box_table(table);
            crate::layout::translate_table(&mut placed, offset.0, offset.1);
            placed
        })
        .collect()
}

/// Which of the four text box layouts an object takes (the renderer's own
/// branches). They differ in the frame the text is placed against, the pen
/// that frame reserves and how the text's origin is written, so none of them
/// can stand for another. Chosen here, once, for both writers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TextBoxLayout {
    /// A lone two-colour gradient box painted as bands (`gradient_drawn`).
    /// Its text frame (`hsT`) is inset by half the effective stroke and
    /// clips the text.
    Gradient,
    /// A text-carrying child of a container whose children share one `svg`.
    /// Its frame clips the text and reserves the declared pen, even when
    /// the line style is NONE.
    SharedChild,
    /// A rectangle filled with a picture, with the declared pen as above.
    ImageFill,
    /// Any other shape. Its own box is the frame, with no pen and no clip of
    /// its own: its CSS border (`ShapeStyle::line_width`) is the edge inside
    /// which the text's origin lies.
    Plain,
}

impl TextBoxLayout {
    /// Its name in reports.
    pub fn name(self) -> &'static str {
        match self {
            TextBoxLayout::Gradient => "gradient",
            TextBoxLayout::SharedChild => "shared-child",
            TextBoxLayout::ImageFill => "image-fill",
            TextBoxLayout::Plain => "plain",
        }
    }

    /// The stroke its frame reserves room for.
    pub fn pen(self, shape: &ShapeStyle) -> HwpUnit {
        match self {
            TextBoxLayout::Gradient => shape.line_width.max(0),
            TextBoxLayout::SharedChild | TextBoxLayout::ImageFill => {
                shape.declared_line_width.max(0)
            }
            TextBoxLayout::Plain => 0,
        }
    }

    /// Whether the frame cuts the text off at its own edge (`overflow`).
    pub fn clips(self) -> bool {
        self != TextBoxLayout::Plain
    }
}

/// The layout the renderer gives an object's text box. `shared_child` is
/// whether the object is a child of a `shared_fill_group`; `image_available`
/// says whether a picture fill's resource can be written.
pub fn text_box_layout(
    object: &PositionedObject,
    shared_child: bool,
    image_available: &impl Fn(&str) -> bool,
) -> TextBoxLayout {
    if shared_child {
        TextBoxLayout::SharedChild
    } else if gradient_drawn(object) {
        TextBoxLayout::Gradient
    } else if object.kind == "rect"
        && object.children.is_empty()
        && object
            .shape
            .as_ref()
            .and_then(|shape| shape.image_fill.as_deref())
            .is_some_and(image_available)
    {
        TextBoxLayout::ImageFill
    } else {
        TextBoxLayout::Plain
    }
}

/// Where a text box's text starts from its frame's origin, as the lengths
/// the page-by-page writer puts on the boxes between them. Each length is a
/// CSS length of its own and the browser snaps each to 1/64 px on its own:
/// the sum of the snapped lengths is where a line lands, which `snap(a) +
/// snap(b)` and `snap(a + b)` do not agree on. Do not add them before
/// snapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextOrigin {
    /// Across: the margin (and half the pen).
    pub left: HwpUnit,
    /// Down: the margin (and half the pen), and in a plain shape the
    /// vertical alignment as well.
    pub top: HwpUnit,
    /// The vertical alignment, when it is a length of its own: a shape with a
    /// pen aligns its text one box further in.
    pub align: Option<HwpUnit>,
}

impl TextOrigin {
    /// The lengths across, each to be snapped alone.
    pub fn x_terms(self) -> [HwpUnit; 1] {
        [self.left]
    }

    /// The lengths down, each to be snapped alone.
    pub fn y_terms(self) -> impl Iterator<Item = HwpUnit> {
        std::iter::once(self.top).chain(self.align)
    }
}

/// A shape's text box as both writers place it: the layout it takes and the
/// origin it puts its text at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextBoxPlan {
    pub layout: TextBoxLayout,
    /// The stroke the frame reserves room for (half of it on each side).
    pub pen: HwpUnit,
    /// The margins, the half pen and the vertical alignment.
    pub text: TextBox,
}

impl TextBoxPlan {
    pub fn new(layout: TextBoxLayout, shape: &ShapeStyle, height: HwpUnit) -> Self {
        let pen = layout.pen(shape);
        TextBoxPlan {
            layout,
            pen,
            text: text_box(shape, height, pen),
        }
    }

    /// How far the text's frame (the clipping `hsT`) sits outside the
    /// declared box: half the pen, so the stroke's centre line lands on the
    /// box's edge.
    pub fn inset(&self) -> HwpUnit {
        self.pen / 2
    }

    /// Where the text starts from the frame's origin.
    pub fn origin(&self) -> TextOrigin {
        match self.layout {
            // The plain shape writes margin and alignment as one length.
            TextBoxLayout::Plain => TextOrigin {
                left: self.text.left,
                top: self.text.top + self.text.align_top,
                align: None,
            },
            _ => TextOrigin {
                left: self.text.left,
                top: self.text.top,
                align: Some(self.text.align_top),
            },
        }
    }

    /// Where the text box's tables start from the frame's origin. They are
    /// laid out with this offset, so a table is never moved by it again.
    pub fn table_offset(&self) -> (HwpUnit, HwpUnit) {
        self.text.table_offset()
    }
}

/// The plan of an object's text box, or `None` for an object with no shape.
pub fn text_box_plan(
    object: &PositionedObject,
    shared_child: bool,
    image_available: &impl Fn(&str) -> bool,
) -> Option<TextBoxPlan> {
    let shape = object.shape.as_deref()?;
    Some(TextBoxPlan::new(
        text_box_layout(object, shared_child, image_available),
        shape,
        object.box_units.height.max(1),
    ))
}

/// Visit every object the layout holds, at any depth, once: the page's
/// floats, the objects set in lines, in cells (and their nested tables), in
/// the text boxes of shapes and among a shape's children. `shared_child` is
/// whether the object is a child of a `shared_fill_group`.
pub fn visit_objects<'a>(
    layout: &'a crate::model::LayoutDocument,
    visit: &mut impl FnMut(&'a PositionedObject, bool),
) {
    fn cells<'a>(table: &'a Table, visit: &mut impl FnMut(&'a PositionedObject, bool)) {
        for cell in &table.cells {
            for paragraph in &cell.paragraphs {
                for item in &paragraph.objects {
                    object(item, false, visit);
                }
            }
            for nested in &cell.tables {
                cells(nested, visit);
            }
        }
    }
    fn object<'a>(
        item: &'a PositionedObject,
        shared_child: bool,
        visit: &mut impl FnMut(&'a PositionedObject, bool),
    ) {
        visit(item, shared_child);
        if let Some(shape) = &item.shape {
            for table in &shape.tables {
                cells(table, visit);
            }
            for paragraph in &shape.paragraphs {
                for child in &paragraph.objects {
                    object(child, false, visit);
                }
            }
        }
        let shared = shared_fill_group(item);
        for child in &item.children {
            object(child, shared, visit);
        }
    }
    for page in &layout.pages {
        for item in &page.objects {
            object(item, false, visit);
        }
        for line in &page.lines {
            for item in &line.inline_objects {
                object(item, false, visit);
            }
        }
        for placed in &page.tables {
            cells(&placed.table, visit);
        }
    }
}

/// Every object with a shape and the plan of its text box, in the order
/// [`visit_objects`] reaches them.
pub fn text_box_plans(
    layout: &crate::model::LayoutDocument,
    image_available: impl Fn(&str) -> bool,
) -> Vec<(&PositionedObject, TextBoxPlan)> {
    let mut out = Vec::new();
    visit_objects(layout, &mut |item, shared_child| {
        if let Some(plan) = text_box_plan(item, shared_child, &image_available) {
            out.push((item, plan));
        }
    });
    out
}

/// Collect local table placements straight from the laid-out objects.
/// The renderer used to paint each object into a discarded string to get
/// these values as a side effect. Nested text boxes are visited as well.
pub fn collect_text_box_tables(
    layout: &crate::model::LayoutDocument,
    image_available: impl Fn(&str) -> bool,
) -> Vec<Table> {
    let mut out = Vec::new();
    for (item, plan) in text_box_plans(layout, image_available) {
        if let Some(shape) = &item.shape {
            out.extend(text_box_tables(shape, plan.table_offset()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::css_mm;
    use crate::model::BoxUnits;

    fn shape() -> ShapeStyle {
        ShapeStyle {
            points: Vec::new(),
            fill: None,
            image_fill: None,
            gradient: Vec::new(),
            gradient_step: 0,
            gradient_angle: 0,
            line_color: "#000000".into(),
            line_width: 0,
            declared_line_width: 0,
            corner_ratio: 0,
            paragraphs: Vec::new(),
            tables: Vec::new(),
            margins: [283; 4],
            vertical_align: "TOP".into(),
        }
    }

    fn object(kind: &str, shape: ShapeStyle) -> PositionedObject {
        PositionedObject {
            id: kind.into(),
            key: kind.into(),
            source_path: String::new(),
            source_anchor: None,
            caption: None,
            equation: None,
            textpos: 0,
            kind: kind.into(),
            box_units: BoxUnits {
                x: 0,
                y: 0,
                width: 17288,
                height: 3690,
            },
            anchor: Default::default(),
            alt: String::new(),
            description: String::new(),
            paragraph_key: String::new(),
            binary_ref: None,
            mime_type: None,
            crop: None,
            img_dim: Default::default(),
            original_size: Default::default(),
            flip_x: false,
            flip_y: false,
            stacking_order: 0,
            shape: Some(Box::new(shape)),
            children: Vec::new(),
        }
    }

    fn gradient() -> ShapeStyle {
        ShapeStyle {
            gradient: vec!["#FFFFFF".into(), "#86AFDC".into()],
            gradient_step: 255,
            line_width: 33,
            declared_line_width: 33,
            ..shape()
        }
    }

    fn picture() -> ShapeStyle {
        ShapeStyle {
            image_fill: Some("image84".into()),
            declared_line_width: 85,
            ..shape()
        }
    }

    const ANY: fn(&str) -> bool = |_| true;
    const NONE: fn(&str) -> bool = |_| false;

    fn layout_of(object: &PositionedObject, shared_child: bool) -> TextBoxLayout {
        text_box_layout(object, shared_child, &ANY)
    }

    #[test]
    fn the_layout_is_the_one_the_renderer_dispatches_to() {
        // A plain shape: no pen, no clip of its own.
        let plain = object("rect", shape());
        assert_eq!(layout_of(&plain, false), TextBoxLayout::Plain);
        assert!(!TextBoxLayout::Plain.clips());
        // A drawn gradient box; it comes before a picture fill.
        let mut both = gradient();
        both.image_fill = Some("image84".into());
        assert_eq!(
            layout_of(&object("rect", both), false),
            TextBoxLayout::Gradient
        );
        // A gradient the renderer cannot draw (its colours are unreadable)
        // falls through to the plain layout, with the plain layout's pen.
        let mut unreadable = gradient();
        unreadable.gradient = vec!["none".into(), "#86AFDC".into()];
        let unreadable = object("rect", unreadable);
        assert!(gradient_box(&unreadable) && !gradient_drawn(&unreadable));
        assert_eq!(layout_of(&unreadable, false), TextBoxLayout::Plain);
        // A picture fill needs its resource.
        let filled = object("rect", picture());
        assert_eq!(layout_of(&filled, false), TextBoxLayout::ImageFill);
        assert_eq!(text_box_layout(&filled, false, &NONE), TextBoxLayout::Plain);
        // A picture on a shape with children is not a picture fill.
        let mut parent = object("rect", picture());
        parent.children.push(object("rect", shape()));
        assert_eq!(layout_of(&parent, false), TextBoxLayout::Plain);
        // A child of a shared-svg container takes its layout whatever it is.
        assert_eq!(layout_of(&plain, true), TextBoxLayout::SharedChild);
        assert_eq!(
            layout_of(&object("rect", gradient()), true),
            TextBoxLayout::SharedChild
        );
        assert!(text_box_plan(&object("pic", shape()), false, &ANY).is_some());
        let mut none = object("pic", shape());
        none.shape = None;
        assert!(text_box_plan(&none, false, &ANY).is_none());
    }

    #[test]
    fn the_pen_is_the_one_the_frame_reserves() {
        let line_none = ShapeStyle {
            line_width: 0,
            declared_line_width: 33,
            ..shape()
        };
        // A shared child and a picture fill reserve the declared pen though
        // the line style is NONE; a lone gradient uses the effective stroke.
        assert_eq!(TextBoxLayout::SharedChild.pen(&line_none), 33);
        assert_eq!(TextBoxLayout::ImageFill.pen(&line_none), 33);
        assert_eq!(TextBoxLayout::Gradient.pen(&line_none), 0);
        assert_eq!(TextBoxLayout::Plain.pen(&line_none), 0);
        assert_eq!(TextBoxLayout::Gradient.pen(&gradient()), 33);
    }

    #[test]
    fn a_pen_moves_the_text_one_box_in_and_a_plain_shape_writes_margin_and_alignment_as_one() {
        let mut centred = gradient();
        centred.vertical_align = "CENTER".into();
        let gradient_plan = TextBoxPlan::new(TextBoxLayout::Gradient, &centred, 3690);
        // Margin 283 and half the pen 16 (the odd unit is lost, as the
        // renderer's inset loses it); the free height splits in two.
        assert_eq!(gradient_plan.inset(), 16);
        assert_eq!(
            gradient_plan.origin(),
            TextOrigin {
                left: 299,
                top: 299,
                align: Some((3690 - 283 - 283) / 2)
            }
        );
        assert_eq!(gradient_plan.origin().x_terms(), [299]);
        assert_eq!(
            gradient_plan.origin().y_terms().collect::<Vec<_>>(),
            [299, 1562]
        );
        assert_eq!(gradient_plan.table_offset(), (299, 299 + 1562));
        assert_eq!(css_mm(299), "1.05mm");

        // The same shape, plain: no pen, and its `hcD` alone carries
        // margin and alignment in one length.
        let plain_plan = TextBoxPlan::new(TextBoxLayout::Plain, &centred, 3690);
        assert_eq!(
            plain_plan.origin(),
            TextOrigin {
                left: 283,
                top: 283 + 1562,
                align: None
            }
        );
        assert_eq!(plain_plan.origin().y_terms().count(), 1);
        assert_eq!(plain_plan.table_offset(), (283, 283 + 1562));
        // The tables' offset is the same sum either way: where their own box
        // starts, never added to the snapped terms again.
        assert_eq!(
            plain_plan.table_offset().1,
            plain_plan.origin().y_terms().sum::<HwpUnit>()
        );
    }

    #[test]
    fn the_origin_terms_do_not_agree_with_their_sum_once_snapped() {
        // Why each term is written (and snapped) on its own: 299 and 1562
        // HWPUNIT are 1.05mm and 5.51mm; snapped to 1/64 px alone they do not
        // add up to their sum's snap.
        let snap = |hwp: HwpUnit| {
            let mm = css_mm(hwp).trim_end_matches("mm").parse::<f64>().unwrap();
            ((mm * 96.0 / 25.4) as f32 as f64 * 64.0).floor() as i64
        };
        assert_ne!(snap(299) + snap(1562), snap(299 + 1562));
    }

    #[test]
    fn picture_plan_maps_against_img_dim_regardless_of_placement() {
        let mut obj = object("pic", shape());
        obj.box_units = BoxUnits {
            x: 0,
            y: 0,
            width: 44626,
            height: 69813,
        };
        obj.original_size = BoxUnits {
            x: 0,
            y: 0,
            width: 18543,
            height: 24583,
        };
        obj.img_dim = BoxUnits {
            x: 0,
            y: 0,
            width: 123960,
            height: 164340,
        };
        obj.crop = Some(BoxUnits {
            x: 0,
            y: 0,
            width: 123960,
            height: 164340,
        });

        // Floating full crop maps against img_dim and fills the display box.
        assert_eq!(
            picture_plan(&obj, false),
            PicturePlan {
                fit: PictureFit::Frame(Frame {
                    left: 0,
                    top: 0,
                    width: 44626,
                    height: 69813,
                }),
                basis: PictureBasis::ImgDim,
            }
        );

        // Inline full crop takes no frame: it fills the box (100% size).
        assert_eq!(
            picture_plan(&obj, true),
            PicturePlan {
                fit: PictureFit::FillBox,
                basis: PictureBasis::WholeCanvas,
            }
        );

        // Inline partial crop scales and shifts against img_dim.
        let mut partial_inline = obj.clone();
        partial_inline.crop = Some(BoxUnits {
            x: 1000,
            y: 2000,
            width: 61980,
            height: 82170,
        });
        assert_eq!(
            picture_plan(&partial_inline, true),
            PicturePlan {
                fit: PictureFit::Frame(Frame {
                    left: -(44626 * 1000 / 61980),
                    top: -(69813 * 2000 / 82170),
                    width: 44626 * 123960 / 61980,
                    height: 69813 * 164340 / 82170,
                }),
                basis: PictureBasis::ImgDim,
            }
        );

        // Floating with invalid/zero img_dim falls back to original_size.
        let mut no_dim = obj.clone();
        no_dim.img_dim = BoxUnits::default();
        no_dim.crop = Some(BoxUnits {
            x: 0,
            y: 0,
            width: 18543,
            height: 24583,
        });
        assert_eq!(
            picture_plan(&no_dim, false),
            PicturePlan {
                fit: PictureFit::Frame(Frame {
                    left: 0,
                    top: 0,
                    width: 44626,
                    height: 69813,
                }),
                basis: PictureBasis::OrgSz,
            }
        );
    }

    #[test]
    fn a_missing_frame_is_a_fit_of_its_own_and_says_why() {
        // The 루이지애나 보도자료's 정책브리핑 picture: an inline picture
        // whose imgDim is written 0x0, with a clip that is not orgSz's size.
        let mut obj = object("pic", shape());
        obj.box_units = BoxUnits {
            x: 0,
            y: 0,
            width: 5944,
            height: 2342,
        };
        obj.original_size = BoxUnits {
            x: 0,
            y: 0,
            width: 14460,
            height: 5700,
        };
        obj.crop = Some(BoxUnits {
            x: 0,
            y: 0,
            width: 45180,
            height: 17760,
        });
        // With no canvas the inline picture fills its box; orgSz is not read
        // in its place (it would shrink the picture to 14460/45180 of the box).
        assert_eq!(
            picture_plan(&obj, true),
            PicturePlan {
                fit: PictureFit::FillBox,
                basis: PictureBasis::NoCanvas,
            }
        );
        // A floating one reads the clip against orgSz (not the same fit).
        assert_eq!(
            picture_plan(&obj, false),
            PicturePlan {
                fit: PictureFit::Frame(Frame {
                    left: 0,
                    top: 0,
                    width: 5944 * 14460 / 45180,
                    height: 2342 * 5700 / 17760,
                }),
                basis: PictureBasis::OrgSz,
            }
        );
        // Without orgSz either, a floating picture keeps its natural size.
        obj.original_size = BoxUnits::default();
        assert_eq!(
            picture_plan(&obj, false),
            PicturePlan {
                fit: PictureFit::Natural,
                basis: PictureBasis::NoCanvas,
            }
        );

        // No clip at all, and a clip without extent.
        let mut unclipped = obj.clone();
        unclipped.crop = None;
        assert_eq!(
            picture_plan(&unclipped, true),
            PicturePlan {
                fit: PictureFit::FillBox,
                basis: PictureBasis::NoClip,
            }
        );
        assert_eq!(
            picture_plan(&unclipped, false),
            PicturePlan {
                fit: PictureFit::Natural,
                basis: PictureBasis::NoClip,
            }
        );
        let mut empty = obj.clone();
        empty.img_dim = BoxUnits {
            x: 0,
            y: 0,
            width: 45180,
            height: 17760,
        };
        empty.crop = Some(BoxUnits {
            x: 0,
            y: 0,
            width: 0,
            height: 17760,
        });
        assert_eq!(
            picture_plan(&empty, true),
            PicturePlan {
                fit: PictureFit::FillBox,
                basis: PictureBasis::EmptyClip,
            }
        );

        // The inline branch that takes no frame is wider than "the clip is
        // exactly the canvas": a clip beyond it is left alone too.
        let mut beyond = empty.clone();
        beyond.crop = Some(BoxUnits {
            x: -10,
            y: 0,
            width: 50000,
            height: 20000,
        });
        assert_eq!(
            picture_plan(&beyond, true),
            PicturePlan {
                fit: PictureFit::FillBox,
                basis: PictureBasis::WholeCanvas,
            }
        );
    }
}
