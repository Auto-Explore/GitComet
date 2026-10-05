use anyhow::Error;
use etagere::euclid::{Point2D, Vector2D};
use lyon::geom::Angle;
use lyon::math::{Vector, vector};
use lyon::path::traits::SvgPathBuilder;
use lyon::path::{ArcFlags, Polygon};
use lyon::tessellation::{
    BuffersBuilder, FillTessellator, FillVertex, StrokeTessellator, StrokeVertex, VertexBuffers,
};

pub use lyon::math::Transform;
pub use lyon::tessellation::{FillOptions, FillRule, StrokeOptions};

use crate::{Bounds, Path, PathVertex, Pixels, Point, point, px};

/// Style of the PathBuilder
pub enum PathStyle {
    /// Stroke style
    Stroke(StrokeOptions),
    /// Fill style
    Fill(FillOptions),
}

/// A [`Path`] builder.
pub struct PathBuilder {
    raw: lyon::path::builder::WithSvg<lyon::path::BuilderImpl>,
    transform: Option<lyon::math::Transform>,
    /// PathStyle of the PathBuilder
    pub style: PathStyle,
    dash_array: Option<Vec<Pixels>>,
}

impl From<lyon::path::Builder> for PathBuilder {
    fn from(builder: lyon::path::Builder) -> Self {
        Self {
            raw: builder.with_svg(),
            ..Default::default()
        }
    }
}

impl From<lyon::path::builder::WithSvg<lyon::path::BuilderImpl>> for PathBuilder {
    fn from(raw: lyon::path::builder::WithSvg<lyon::path::BuilderImpl>) -> Self {
        Self {
            raw,
            ..Default::default()
        }
    }
}

impl From<lyon::math::Point> for Point<Pixels> {
    fn from(p: lyon::math::Point) -> Self {
        point(px(p.x), px(p.y))
    }
}

impl From<Point<Pixels>> for lyon::math::Point {
    fn from(p: Point<Pixels>) -> Self {
        lyon::math::point(p.x.0, p.y.0)
    }
}

impl From<Point<Pixels>> for Vector {
    fn from(p: Point<Pixels>) -> Self {
        vector(p.x.0, p.y.0)
    }
}

impl From<Point<Pixels>> for Point2D<f32, Pixels> {
    fn from(p: Point<Pixels>) -> Self {
        Point2D::new(p.x.0, p.y.0)
    }
}

impl Default for PathBuilder {
    fn default() -> Self {
        Self {
            raw: lyon::path::Path::builder().with_svg(),
            style: PathStyle::Fill(FillOptions::default()),
            transform: None,
            dash_array: None,
        }
    }
}

impl PathBuilder {
    /// Creates a new [`PathBuilder`] to build a Stroke path.
    pub fn stroke(width: Pixels) -> Self {
        Self {
            style: PathStyle::Stroke(StrokeOptions::default().with_line_width(width.0)),
            ..Self::default()
        }
    }

    /// Creates a new [`PathBuilder`] to build a Fill path.
    pub fn fill() -> Self {
        Self::default()
    }

    /// Sets the style of the [`PathBuilder`].
    pub fn with_style(self, style: PathStyle) -> Self {
        Self { style, ..self }
    }

    /// Sets the dash array of the [`PathBuilder`].
    ///
    /// [MDN](https://developer.mozilla.org/en-US/docs/Web/SVG/Reference/Attribute/stroke-dasharray)
    pub fn dash_array(mut self, dash_array: &[Pixels]) -> Self {
        // If an odd number of values is provided, then the list of values is repeated to yield an even number of values.
        // Thus, 5,3,2 is equivalent to 5,3,2,5,3,2.
        let array = if dash_array.len() % 2 == 1 {
            let mut new_dash_array = dash_array.to_vec();
            new_dash_array.extend_from_slice(dash_array);
            new_dash_array
        } else {
            dash_array.to_vec()
        };

        self.dash_array = Some(array);
        self
    }

    /// Move the current point to the given point.
    #[inline]
    pub fn move_to(&mut self, to: Point<Pixels>) {
        self.raw.move_to(to.into());
    }

    /// Draw a straight line from the current point to the given point.
    #[inline]
    pub fn line_to(&mut self, to: Point<Pixels>) {
        self.raw.line_to(to.into());
    }

    /// Draw a curve from the current point to the given point, using the given control point.
    #[inline]
    pub fn curve_to(&mut self, to: Point<Pixels>, ctrl: Point<Pixels>) {
        self.raw.quadratic_bezier_to(ctrl.into(), to.into());
    }

    /// Adds a cubic Bézier to the [`Path`] given its two control points
    /// and its end point.
    #[inline]
    pub fn cubic_bezier_to(
        &mut self,
        to: Point<Pixels>,
        control_a: Point<Pixels>,
        control_b: Point<Pixels>,
    ) {
        self.raw
            .cubic_bezier_to(control_a.into(), control_b.into(), to.into());
    }

    /// Adds an elliptical arc.
    pub fn arc_to(
        &mut self,
        radii: Point<Pixels>,
        x_rotation: Pixels,
        large_arc: bool,
        sweep: bool,
        to: Point<Pixels>,
    ) {
        self.raw.arc_to(
            radii.into(),
            Angle::degrees(x_rotation.into()),
            ArcFlags { large_arc, sweep },
            to.into(),
        );
    }

    /// Equivalent to `arc_to` in relative coordinates.
    pub fn relative_arc_to(
        &mut self,
        radii: Point<Pixels>,
        x_rotation: Pixels,
        large_arc: bool,
        sweep: bool,
        to: Point<Pixels>,
    ) {
        self.raw.relative_arc_to(
            radii.into(),
            Angle::degrees(x_rotation.into()),
            ArcFlags { large_arc, sweep },
            to.into(),
        );
    }

    /// Adds a polygon.
    pub fn add_polygon(&mut self, points: &[Point<Pixels>], closed: bool) {
        let points = points.iter().copied().map(|p| p.into()).collect::<Vec<_>>();
        self.raw.add_polygon(Polygon {
            points: points.as_ref(),
            closed,
        });
    }

    /// Close the current sub-path.
    #[inline]
    pub fn close(&mut self) {
        self.raw.close();
    }

    /// Applies a transform to the path.
    #[inline]
    pub fn transform(&mut self, transform: Transform) {
        self.transform = Some(transform);
    }

    /// Applies a translation to the path.
    #[inline]
    pub fn translate(&mut self, to: Point<Pixels>) {
        if let Some(transform) = self.transform {
            self.transform = Some(transform.then_translate(Vector2D::new(to.x.0, to.y.0)));
        } else {
            self.transform = Some(Transform::translation(to.x.0, to.y.0))
        }
    }

    /// Applies a scale to the path.
    #[inline]
    pub fn scale(&mut self, scale: f32) {
        if let Some(transform) = self.transform {
            self.transform = Some(transform.then_scale(scale, scale));
        } else {
            self.transform = Some(Transform::scale(scale, scale));
        }
    }

    /// Applies a rotation to the path.
    ///
    /// The `angle` is in degrees value in the range 0.0 to 360.0.
    #[inline]
    pub fn rotate(&mut self, angle: f32) {
        let radians = angle.to_radians();
        if let Some(transform) = self.transform {
            self.transform = Some(transform.then_rotate(Angle::radians(radians)));
        } else {
            self.transform = Some(Transform::rotation(Angle::radians(radians)));
        }
    }

    /// Builds into a [`Path`].
    #[inline]
    pub fn build(self) -> Result<Path<Pixels>, Error> {
        let path = if let Some(transform) = self.transform {
            self.raw.build().transformed(&transform)
        } else {
            self.raw.build()
        };

        match self.style {
            PathStyle::Stroke(options) => Self::tessellate_stroke(self.dash_array, &path, &options),
            PathStyle::Fill(options) => Self::tessellate_fill(&path, &options),
        }
    }

    fn tessellate_fill(
        path: &lyon::path::Path,
        options: &FillOptions,
    ) -> Result<Path<Pixels>, Error> {
        // Will contain the result of the tessellation.
        let mut buf: VertexBuffers<lyon::math::Point, u16> = VertexBuffers::new();
        let mut tessellator = FillTessellator::new();

        // Compute the tessellation.
        tessellator.tessellate_path(
            path,
            options,
            &mut BuffersBuilder::new(&mut buf, |vertex: FillVertex| vertex.position()),
        )?;

        Ok(Self::build_path(buf))
    }

    fn tessellate_stroke(
        dash_array: Option<Vec<Pixels>>,
        path: &lyon::path::Path,
        options: &StrokeOptions,
    ) -> Result<Path<Pixels>, Error> {
        let path = if let Some(dash_array) = dash_array {
            let measurements = lyon::algorithms::measure::PathMeasurements::from_path(path, 0.01);
            let mut sampler = measurements
                .create_sampler(path, lyon::algorithms::measure::SampleType::Normalized);
            let mut builder = lyon::path::Path::builder();

            let total_length = sampler.length();
            let dash_array_len = dash_array.len();
            let mut pos = 0.;
            let mut dash_index = 0;
            while pos < total_length {
                let dash_length = dash_array[dash_index % dash_array_len].0;
                let next_pos = (pos + dash_length).min(total_length);
                if dash_index % 2 == 0 {
                    let start = pos / total_length;
                    let end = next_pos / total_length;
                    sampler.split_range(start..end, &mut builder);
                }
                pos = next_pos;
                dash_index += 1;
            }

            &builder.build()
        } else {
            path
        };

        // Will contain the result of the tessellation.
        let mut buf: VertexBuffers<lyon::math::Point, u16> = VertexBuffers::new();
        let mut tessellator = StrokeTessellator::new();

        // Compute the tessellation.
        tessellator.tessellate_path(
            path,
            options,
            &mut BuffersBuilder::new(&mut buf, |vertex: StrokeVertex| vertex.position()),
        )?;

        Ok(Self::build_path(buf))
    }

    /// Builds a [`Path`] from a [`lyon::tessellation::VertexBuffers`].
    pub fn build_path(buf: VertexBuffers<lyon::math::Point, u16>) -> Path<Pixels> {
        if buf.vertices.is_empty() {
            return Path::new(Point::default());
        }

        let first_point = buf.vertices[0];
        let mut min_x = first_point.x;
        let mut min_y = first_point.y;
        let mut max_x = first_point.x;
        let mut max_y = first_point.y;

        let mut path = Path::new(first_point.into());
        path.vertices.reserve(buf.indices.len());
        for triangle in buf.indices.chunks_exact(3) {
            let i0 = triangle[0] as usize;
            let i1 = triangle[1] as usize;
            let i2 = triangle[2] as usize;

            let v0 = buf.vertices[i0];
            let v1 = buf.vertices[i1];
            let v2 = buf.vertices[i2];

            min_x = min_x.min(v0.x).min(v1.x).min(v2.x);
            min_y = min_y.min(v0.y).min(v1.y).min(v2.y);
            max_x = max_x.max(v0.x).max(v1.x).max(v2.x);
            max_y = max_y.max(v0.y).max(v1.y).max(v2.y);

            let st = point(0., 1.);
            path.vertices.push(PathVertex {
                xy_position: v0.into(),
                st_position: st,
                content_mask: Default::default(),
            });
            path.vertices.push(PathVertex {
                xy_position: v1.into(),
                st_position: st,
                content_mask: Default::default(),
            });
            path.vertices.push(PathVertex {
                xy_position: v2.into(),
                st_position: st,
                content_mask: Default::default(),
            });
        }

        path.bounds =
            Bounds::from_corners(point(px(min_x), px(min_y)), point(px(max_x), px(max_y)));

        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_macros::perf;
    use std::hint::black_box;

    #[test]
    fn build_path_handles_empty_buffers() {
        let path = PathBuilder::build_path(VertexBuffers::new());

        assert!(path.vertices.is_empty());
        assert_eq!(
            path.bounds,
            Bounds::new(Point::default(), Default::default())
        );
    }

    #[test]
    fn build_path_handles_vertices_without_indices() {
        let mut buf: VertexBuffers<lyon::math::Point, u16> = VertexBuffers::new();
        buf.vertices.push(lyon::math::point(-2.0, 3.0));
        buf.vertices.push(lyon::math::point(8.0, 9.0));

        let path = PathBuilder::build_path(buf);

        assert!(path.vertices.is_empty());
        assert_eq!(
            path.bounds,
            Bounds::from_corners(point(px(-2.0), px(3.0)), point(px(-2.0), px(3.0)))
        );
    }

    #[test]
    fn build_path_computes_bounds_and_vertex_metadata() {
        let mut buf: VertexBuffers<lyon::math::Point, u16> = VertexBuffers::new();
        buf.vertices.extend_from_slice(&[
            lyon::math::point(-2.0, 3.0),
            lyon::math::point(4.0, -1.0),
            lyon::math::point(1.0, 5.0),
            lyon::math::point(6.0, 7.0),
        ]);
        buf.indices.extend_from_slice(&[0, 1, 2, 1, 2, 3]);

        let path = PathBuilder::build_path(buf);

        assert_eq!(path.vertices.len(), 6);
        assert_eq!(
            path.bounds,
            Bounds::from_corners(point(px(-2.0), px(-1.0)), point(px(6.0), px(7.0)))
        );
        for vertex in &path.vertices {
            assert_eq!(vertex.st_position, point(0., 1.));
            assert_eq!(vertex.content_mask.bounds, Bounds::default());
        }
        assert_eq!(path.vertices[0].xy_position, point(px(-2.0), px(3.0)));
        assert_eq!(path.vertices[5].xy_position, point(px(6.0), px(7.0)));
    }

    #[test]
    fn build_path_ignores_trailing_incomplete_index_group() {
        let mut buf: VertexBuffers<lyon::math::Point, u16> = VertexBuffers::new();
        buf.vertices.extend_from_slice(&[
            lyon::math::point(0.0, 0.0),
            lyon::math::point(1.0, 0.0),
            lyon::math::point(0.0, 1.0),
            lyon::math::point(100.0, 100.0),
        ]);
        buf.indices.extend_from_slice(&[0, 1, 2, 3]);

        let path = PathBuilder::build_path(buf);

        assert_eq!(path.vertices.len(), 3);
        assert_eq!(
            path.bounds,
            Bounds::from_corners(point(px(0.0), px(0.0)), point(px(1.0), px(1.0)))
        );
    }

    #[perf(important)]
    fn perf_build_path_many_triangles() {
        const TRIANGLES: usize = 4_096;

        let mut buf: VertexBuffers<lyon::math::Point, u16> = VertexBuffers::new();
        buf.vertices.reserve(TRIANGLES * 3);
        buf.indices.reserve(TRIANGLES * 3);

        for ix in 0..TRIANGLES {
            let base = (ix * 3) as u16;
            let x = (ix % 128) as f32;
            let y = (ix / 128) as f32;

            buf.vertices.push(lyon::math::point(x, y));
            buf.vertices.push(lyon::math::point(x + 1.0, y));
            buf.vertices.push(lyon::math::point(x, y + 1.0));
            buf.indices.extend_from_slice(&[base, base + 1, base + 2]);
        }

        let path = PathBuilder::build_path(buf);

        assert_eq!(path.vertices.len(), TRIANGLES * 3);
        black_box(path.bounds);
        black_box(path.vertices.len());
    }
}
