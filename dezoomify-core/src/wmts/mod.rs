//! Pure discovery for Web Map Tile Service capabilities documents.

use std::sync::Arc;

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;

use crate::Vec2d;
use crate::core::{
    CatalogEntry, DezoomerSpec, DiscoveryError, DiscoveryMatch, Grid, ImageCatalog,
    ImageDescriptor, LevelDescriptor, Request, StableId, resolve_relative,
};

const RADIUS: f64 = 6_378_137.0;
const HALF_SIZE: f64 = std::f64::consts::PI * RADIUS;
const METRES_PER_PIXEL: f64 = 0.28e-3;

pub const SPEC: DezoomerSpec = DezoomerSpec::new("wmts", &[DiscoveryMatch::Any.extract(catalog)])
    .recognizing(is_wmts_url, "not a WMTS capabilities URL")
    .preferring(|uri| uri.to_ascii_lowercase().contains("wmts"));

fn is_wmts_url(uri: &str) -> bool {
    uri.to_ascii_lowercase().contains("wmts")
}

fn catalog(url: &str, bytes: &[u8]) -> Result<ImageCatalog, DiscoveryError> {
    let document = parse_document(bytes)?;
    let context = parse_context(url, &document)?;
    let levels = build_levels(&context)?;
    if levels.is_empty() {
        return Err(DiscoveryError::Session("WMTS has no tile matrices".into()));
    }
    Ok(ImageCatalog::new([CatalogEntry::Ready(ImageDescriptor {
        id: StableId::new("wmts:image"),
        title: Some(context.layer_name),
        format: StableId::new("wmts"),
        levels,
        ..Default::default()
    })]))
}

#[derive(Debug)]
struct XmlElement {
    name: String,
    attributes: Vec<XmlAttribute>,
    children: Vec<XmlElement>,
    text: String,
}

#[derive(Debug)]
struct XmlAttribute {
    name: String,
    value: String,
}

fn parse_document(bytes: &[u8]) -> Result<XmlElement, DiscoveryError> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut root = None;
    let mut stack = Vec::new();

    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| DiscoveryError::Session(format!("invalid WMTS XML: {error}")))?;
        match event {
            Event::Start(start) => stack.push(element_from_start(&start)?),
            Event::Empty(start) => {
                append_element(&mut root, &mut stack, element_from_start(&start)?)?;
            }
            Event::End(_) => {
                let element = stack.pop().ok_or_else(|| {
                    DiscoveryError::Session("invalid WMTS XML: unmatched closing element".into())
                })?;
                append_element(&mut root, &mut stack, element)?;
            }
            Event::Text(text) => {
                let unescaped = quick_xml::escape::unescape(&text).map_err(|error| {
                    DiscoveryError::Session(format!("invalid WMTS text escape: {error}"))
                })?;
                append_text(&mut stack, unescaped.as_ref())?;
            }
            Event::CData(text) => {
                append_text(&mut stack, &text)?;
            }
            Event::GeneralRef(reference) => {
                let escaped = format!("&{};", reference.as_ref());
                let unescaped = quick_xml::escape::unescape(&escaped).map_err(|error| {
                    DiscoveryError::Session(format!("invalid WMTS reference: {error}"))
                })?;
                append_text(&mut stack, unescaped.as_ref())?;
            }
            Event::Eof => break,
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) | Event::DocType(_) => {}
        }
        buffer.clear();
    }

    if !stack.is_empty() {
        return Err(DiscoveryError::Session(
            "invalid WMTS XML: unclosed element".into(),
        ));
    }
    root.ok_or_else(|| DiscoveryError::Session("invalid WMTS XML: no document element".into()))
}

fn element_from_start(start: &BytesStart<'_>) -> Result<XmlElement, DiscoveryError> {
    let name = start.local_name().into_inner().to_string();
    let mut attributes = Vec::new();
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|error| {
            DiscoveryError::Session(format!("invalid WMTS XML attribute: {error}"))
        })?;
        let name = attribute.key.local_name().into_inner().to_string();
        let value = quick_xml::escape::unescape(&attribute.value)
            .map_err(|error| {
                DiscoveryError::Session(format!("invalid WMTS XML attribute value: {error}"))
            })?
            .into_owned();
        attributes.push(XmlAttribute { name, value });
    }
    Ok(XmlElement {
        name,
        attributes,
        children: Vec::new(),
        text: String::new(),
    })
}

#[allow(clippy::ptr_arg)]
fn append_element(
    root: &mut Option<XmlElement>,
    stack: &mut Vec<XmlElement>,
    element: XmlElement,
) -> Result<(), DiscoveryError> {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(element);
    } else if root.is_some() {
        return Err(DiscoveryError::Session(
            "invalid WMTS XML: multiple document elements".into(),
        ));
    } else {
        *root = Some(element);
    }
    Ok(())
}

fn append_text(stack: &mut [XmlElement], text: &str) -> Result<(), DiscoveryError> {
    if let Some(element) = stack.last_mut() {
        element.text.push_str(text);
        Ok(())
    } else if text.trim().is_empty() {
        Ok(())
    } else {
        Err(DiscoveryError::Session(
            "invalid WMTS XML: text outside document element".into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CoordinateReference {
    Geographic,
    WebMercator,
}

#[derive(Clone, Copy, Debug)]
struct Bounds {
    left: f64,
    bottom: f64,
    right: f64,
    top: f64,
}

#[derive(Clone, Debug)]
struct MatrixLimit {
    matrix: String,
    minimum_column: u32,
    maximum_column: u32,
    minimum_row: u32,
    maximum_row: u32,
}

#[derive(Clone, Debug)]
struct TileMatrix {
    identifier: String,
    scale_denominator: f64,
    top_left: (f64, f64),
    tile_size: Vec2d,
    matrix_size: Vec2d,
}

#[derive(Debug)]
struct MatrixSet {
    identifier: String,
    matrices: Vec<TileMatrix>,
}

#[derive(Debug)]
struct MatrixSetLink {
    matrix_set: String,
    limits: Vec<MatrixLimit>,
}

struct WmtsContext {
    layer_name: String,
    template: Arc<str>,
    matrix_set_name: String,
    style: String,
    bounds: Option<Bounds>,
    matrices: Vec<TileMatrix>,
    limits: Vec<MatrixLimit>,
}

fn parse_context(url: &str, document: &XmlElement) -> Result<WmtsContext, DiscoveryError> {
    let contents = find_descendant(document, "Contents").unwrap_or(document);
    let mut matrix_sets = Vec::new();
    for element in descendants_named(contents, "TileMatrixSet") {
        if let Ok(matrix_set) = parse_matrix_set(element) {
            matrix_sets.push(matrix_set);
        }
    }
    if matrix_sets.is_empty() {
        return Err(DiscoveryError::Session(
            "WMTS has no supported tile matrix set".into(),
        ));
    }

    let mut last_error = None;
    for layer in descendants_named(contents, "Layer") {
        match context_for_layer(url, layer, &matrix_sets) {
            Ok(context) => return Ok(context),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error
        .unwrap_or_else(|| DiscoveryError::Session("WMTS capabilities has no layer".into())))
}

fn parse_matrix_set(element: &XmlElement) -> Result<MatrixSet, DiscoveryError> {
    let identifier = required_text(element, "Identifier", "matrix set identifier")?;
    let supported_crs = required_text(element, "SupportedCRS", "matrix set CRS")?;
    let reference = coordinate_reference(&supported_crs).ok_or_else(|| {
        DiscoveryError::Session(format!(
            "unsupported WMTS coordinate reference system: {supported_crs}"
        ))
    })?;
    if reference != CoordinateReference::WebMercator {
        return Err(DiscoveryError::Session(
            "WMTS tile matrix set is not Web Mercator".into(),
        ));
    }
    let matrices = element
        .children_named("TileMatrix")
        .map(parse_matrix)
        .collect::<Result<Vec<_>, _>>()?;
    if matrices.is_empty() {
        return Err(DiscoveryError::Session(
            "WMTS tile matrix set has no tile matrices".into(),
        ));
    }
    Ok(MatrixSet {
        identifier,
        matrices,
    })
}

fn parse_matrix(element: &XmlElement) -> Result<TileMatrix, DiscoveryError> {
    let identifier = required_text(element, "Identifier", "matrix identifier")?;
    let scale_denominator = positive_number(
        &required_text(element, "ScaleDenominator", "scale denominator")?,
        "scale denominator",
    )?;
    let top_left = coordinates(
        &required_text(element, "TopLeftCorner", "top-left corner")?,
        "top-left corner",
    )?;
    let tile_size = Vec2d {
        x: positive_integer(
            &required_text(element, "TileWidth", "tile width")?,
            "tile width",
        )?,
        y: positive_integer(
            &required_text(element, "TileHeight", "tile height")?,
            "tile height",
        )?,
    };
    let matrix_size = Vec2d {
        x: positive_integer(
            &required_text(element, "MatrixWidth", "matrix width")?,
            "matrix width",
        )?,
        y: positive_integer(
            &required_text(element, "MatrixHeight", "matrix height")?,
            "matrix height",
        )?,
    };
    Ok(TileMatrix {
        identifier,
        scale_denominator,
        top_left,
        tile_size,
        matrix_size,
    })
}

fn context_for_layer(
    url: &str,
    layer: &XmlElement,
    matrix_sets: &[MatrixSet],
) -> Result<WmtsContext, DiscoveryError> {
    let layer_name = required_text(layer, "Identifier", "layer identifier")?;
    let template = resource_template(layer)?;
    validate_template(&template)?;
    let style = layer_style(layer);
    let bounds = parse_layer_bounds(layer)?;
    let links = layer
        .children_named("TileMatrixSetLink")
        .map(parse_matrix_set_link)
        .collect::<Result<Vec<_>, _>>()?;

    let selected = if links.is_empty() {
        matrix_sets
            .iter()
            .next()
            .map(|matrix_set| (matrix_set, None))
    } else {
        links.iter().find_map(|link| {
            matrix_sets
                .iter()
                .find(|matrix_set| matrix_set.identifier == link.matrix_set)
                .map(|matrix_set| (matrix_set, Some(link)))
        })
    }
    .ok_or_else(|| {
        DiscoveryError::Session("WMTS layer has no supported linked tile matrix set".into())
    })?;

    let projected_bounds = bounds.map(project_bounds).transpose()?;
    let (matrix_set, link) = selected;
    Ok(WmtsContext {
        layer_name,
        template: resolve_template(url, &template).into(),
        matrix_set_name: matrix_set.identifier.clone(),
        style,
        bounds: projected_bounds,
        matrices: matrix_set.matrices.clone(),
        limits: link.map_or_else(Vec::new, |link| link.limits.clone()),
    })
}

fn resolve_template(base: &str, template: &str) -> String {
    resolve_relative(base, template)
        .replace("%7B", "{")
        .replace("%7b", "{")
        .replace("%7D", "}")
        .replace("%7d", "}")
}

fn parse_layer_bounds(layer: &XmlElement) -> Result<Option<LayerBounds>, DiscoveryError> {
    let projected = layer.children_named("BoundingBox").next();
    let geographic = layer.children_named("WGS84BoundingBox").next();
    let Some(element) = projected.or(geographic) else {
        return Ok(None);
    };
    let reference = if same_name(&element.name, "WGS84BoundingBox") {
        CoordinateReference::Geographic
    } else {
        element
            .attribute("crs")
            .and_then(coordinate_reference)
            .ok_or_else(|| DiscoveryError::Session("unsupported WMTS bounding-box CRS".into()))?
    };
    let lower = coordinates(
        &required_text(element, "LowerCorner", "bounding-box lower corner")?,
        "bounding-box lower corner",
    )?;
    let upper = coordinates(
        &required_text(element, "UpperCorner", "bounding-box upper corner")?,
        "bounding-box upper corner",
    )?;
    if lower.0 > upper.0 || lower.1 > upper.1 {
        return Err(DiscoveryError::Session(
            "WMTS bounding box has invalid corner order".into(),
        ));
    }
    Ok(Some(LayerBounds {
        reference,
        lower,
        upper,
    }))
}

#[derive(Clone, Copy, Debug)]
struct LayerBounds {
    reference: CoordinateReference,
    lower: (f64, f64),
    upper: (f64, f64),
}

fn project_bounds(bounds: LayerBounds) -> Result<Bounds, DiscoveryError> {
    let (left, bottom) = project_coordinate(bounds.lower.0, bounds.lower.1, bounds.reference)?;
    let (right, top) = project_coordinate(bounds.upper.0, bounds.upper.1, bounds.reference)?;
    Ok(Bounds {
        left,
        bottom,
        right,
        top,
    })
}

fn parse_matrix_set_link(element: &XmlElement) -> Result<MatrixSetLink, DiscoveryError> {
    let matrix_set = required_text(element, "TileMatrixSet", "linked matrix set")?;
    let limits = element
        .children_named("TileMatrixSetLimits")
        .next()
        .map(|limits| {
            limits
                .children_named("TileMatrixLimits")
                .map(parse_matrix_limit)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    Ok(MatrixSetLink { matrix_set, limits })
}

fn parse_matrix_limit(element: &XmlElement) -> Result<MatrixLimit, DiscoveryError> {
    let matrix = required_text(element, "TileMatrix", "matrix limit identifier")?;
    let minimum_column = nonnegative_integer(
        &required_text(element, "MinTileCol", "minimum tile column")?,
        "minimum tile column",
    )?;
    let maximum_column = nonnegative_integer(
        &required_text(element, "MaxTileCol", "maximum tile column")?,
        "maximum tile column",
    )?;
    let minimum_row = nonnegative_integer(
        &required_text(element, "MinTileRow", "minimum tile row")?,
        "minimum tile row",
    )?;
    let maximum_row = nonnegative_integer(
        &required_text(element, "MaxTileRow", "maximum tile row")?,
        "maximum tile row",
    )?;
    if minimum_column > maximum_column || minimum_row > maximum_row {
        return Err(DiscoveryError::Session(
            "WMTS tile matrix limits have invalid ranges".into(),
        ));
    }
    Ok(MatrixLimit {
        matrix,
        minimum_column,
        maximum_column,
        minimum_row,
        maximum_row,
    })
}

fn resource_template(layer: &XmlElement) -> Result<String, DiscoveryError> {
    let resources: Vec<_> = layer.children_named("ResourceURL").collect();
    let tile_resources: Vec<_> = resources
        .iter()
        .copied()
        .filter(|resource| {
            resource
                .attribute("resourceType")
                .is_none_or(|kind| kind.eq_ignore_ascii_case("tile"))
        })
        .collect();
    let formats: Vec<_> = layer
        .children_named("Format")
        .map(text_content)
        .filter(|format| !format.is_empty())
        .collect();
    let candidates = if tile_resources.is_empty() {
        resources
    } else {
        tile_resources
    };
    candidates
        .iter()
        .find(|resource| {
            resource.attribute("format").is_some_and(|format| {
                formats
                    .iter()
                    .any(|layer_format| layer_format.eq_ignore_ascii_case(format))
            })
        })
        .or_else(|| candidates.first())
        .and_then(|resource| resource.attribute("template"))
        .filter(|template| !template.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| DiscoveryError::Session("WMTS layer has no tile URL template".into()))
}

fn layer_style(layer: &XmlElement) -> String {
    let styles: Vec<_> = layer.children_named("Style").collect();
    styles
        .iter()
        .find(|style| {
            style
                .attribute("isDefault")
                .is_some_and(|value| value.eq_ignore_ascii_case("true") || value == "1")
        })
        .or_else(|| styles.first())
        .and_then(|style| style.children_named("Identifier").next())
        .map(text_content)
        .filter(|style| !style.is_empty())
        .unwrap_or_else(|| "default".into())
}

fn build_levels(context: &WmtsContext) -> Result<Vec<LevelDescriptor>, DiscoveryError> {
    context
        .matrices
        .iter()
        .enumerate()
        .map(|(ordinal, matrix)| {
            let ((min_column, max_column), (min_row, max_row)) = tile_ranges(context, matrix)?;
            let columns = count_between(min_column, max_column)?;
            let rows = count_between(min_row, max_row)?;
            let width = u64::from(columns)
                .checked_mul(u64::from(matrix.tile_size.x))
                .and_then(|size| u32::try_from(size).ok())
                .ok_or_else(|| DiscoveryError::Session("WMTS image width is too large".into()))?;
            let height = u64::from(rows)
                .checked_mul(u64::from(matrix.tile_size.y))
                .and_then(|size| u32::try_from(size).ok())
                .ok_or_else(|| DiscoveryError::Session("WMTS image height is too large".into()))?;
            let template = Arc::clone(&context.template);
            let matrix_set = context.matrix_set_name.clone();
            let matrix_identifier = matrix.identifier.clone();
            let style = context.style.clone();
            let source = Grid::with_requests(
                StableId::new(format!("wmts:{ordinal}")),
                Vec2d {
                    x: width,
                    y: height,
                },
                matrix.tile_size,
                Vec2d::default(),
                move |tile| {
                    render_template(
                        &template,
                        &matrix_set,
                        &matrix_identifier,
                        &style,
                        min_column + tile.coord.column,
                        min_row + tile.coord.row,
                    )
                },
            )
            .map_err(|error| DiscoveryError::Session(format!("invalid WMTS grid: {error}")))?;
            Ok(LevelDescriptor::new(source)
                .with_title(Some(format!("WMTS matrix {}", matrix.identifier))))
        })
        .collect()
}

type TileRange = (u32, u32);

fn tile_ranges(
    context: &WmtsContext,
    matrix: &TileMatrix,
) -> Result<(TileRange, TileRange), DiscoveryError> {
    let mut columns = (0, matrix.matrix_size.x - 1);
    let mut rows = (0, matrix.matrix_size.y - 1);
    if let Some(bounds) = context.bounds {
        let x_span = f64::from(matrix.tile_size.x) * matrix.scale_denominator * METRES_PER_PIXEL;
        let y_span = f64::from(matrix.tile_size.y) * matrix.scale_denominator * METRES_PER_PIXEL;
        if !x_span.is_finite() || !y_span.is_finite() || x_span <= 0.0 || y_span <= 0.0 {
            return Err(DiscoveryError::Session(
                "WMTS matrix has an invalid tile span".into(),
            ));
        }
        let minimum_column = floor_index((bounds.left - matrix.top_left.0) / x_span)?;
        let maximum_column = floor_index((bounds.right - matrix.top_left.0) / x_span)?;
        let minimum_row = floor_index((matrix.top_left.1 - bounds.top) / y_span)?;
        let maximum_row = floor_index((matrix.top_left.1 - bounds.bottom) / y_span)?;
        columns = clamp_range(minimum_column, maximum_column, matrix.matrix_size.x)?;
        rows = clamp_range(minimum_row, maximum_row, matrix.matrix_size.y)?;
    }
    if let Some(limit) = context
        .limits
        .iter()
        .find(|limit| limit.matrix == matrix.identifier)
    {
        columns.0 = columns.0.max(limit.minimum_column);
        columns.1 = columns.1.min(limit.maximum_column);
        rows.0 = rows.0.max(limit.minimum_row);
        rows.1 = rows.1.min(limit.maximum_row);
    }
    if columns.0 > columns.1 || rows.0 > rows.1 {
        return Err(DiscoveryError::Session(
            "WMTS tile matrix has no tiles in the layer extent".into(),
        ));
    }
    Ok((columns, rows))
}

fn clamp_range(minimum: i64, maximum: i64, matrix_size: u32) -> Result<(u32, u32), DiscoveryError> {
    let maximum_index = i64::from(matrix_size - 1);
    if maximum < 0 || minimum > maximum_index || minimum > maximum {
        return Err(DiscoveryError::Session(
            "WMTS bounding box is outside its tile matrix".into(),
        ));
    }
    let minimum = minimum.max(0).min(maximum_index);
    let maximum = maximum.max(0).min(maximum_index);
    Ok((
        u32::try_from(minimum)
            .map_err(|_| DiscoveryError::Session("WMTS tile coordinate is out of range".into()))?,
        u32::try_from(maximum)
            .map_err(|_| DiscoveryError::Session("WMTS tile coordinate is out of range".into()))?,
    ))
}

fn find_descendant<'a>(element: &'a XmlElement, name: &str) -> Option<&'a XmlElement> {
    if same_name(&element.name, name) {
        return Some(element);
    }
    element
        .children
        .iter()
        .find_map(|child| find_descendant(child, name))
}

fn descendants_named<'a>(element: &'a XmlElement, name: &str) -> Vec<&'a XmlElement> {
    let mut descendants = Vec::new();
    collect_descendants(element, name, &mut descendants);
    descendants
}

fn collect_descendants<'a>(
    element: &'a XmlElement,
    name: &str,
    descendants: &mut Vec<&'a XmlElement>,
) {
    if same_name(&element.name, name) {
        descendants.push(element);
    }
    for child in &element.children {
        collect_descendants(child, name, descendants);
    }
}

impl XmlElement {
    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a XmlElement> + 'a {
        self.children
            .iter()
            .filter(move |child| same_name(&child.name, name))
    }

    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|attribute| same_name(&attribute.name, name))
            .map(|attribute| attribute.value.as_str())
    }
}

fn same_name(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn text_content(element: &XmlElement) -> String {
    let mut text = element.text.clone();
    for child in &element.children {
        text.push_str(&text_content(child));
    }
    text.trim().to_owned()
}

fn required_text(element: &XmlElement, name: &str, label: &str) -> Result<String, DiscoveryError> {
    element
        .children_named(name)
        .next()
        .map(text_content)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DiscoveryError::Session(format!("WMTS has no {label}")))
}

fn coordinate_reference(value: &str) -> Option<CoordinateReference> {
    let value = value.trim().to_ascii_lowercase();
    if value.contains("crs84") || value.ends_with("4326") {
        Some(CoordinateReference::Geographic)
    } else if value.ends_with("3857") {
        Some(CoordinateReference::WebMercator)
    } else {
        None
    }
}

fn project_coordinate(
    x: f64,
    y: f64,
    reference: CoordinateReference,
) -> Result<(f64, f64), DiscoveryError> {
    if !x.is_finite() || !y.is_finite() {
        return Err(DiscoveryError::Session(
            "WMTS bounding box has non-finite coordinates".into(),
        ));
    }
    match reference {
        CoordinateReference::WebMercator => Ok((x, y)),
        CoordinateReference::Geographic => {
            if !(-180.0..=180.0).contains(&x) || !(-90.0..=90.0).contains(&y) || y.abs() >= 90.0 {
                return Err(DiscoveryError::Session(
                    "invalid WMTS geographic bounding box".into(),
                ));
            }
            let projected_y = RADIUS * (std::f64::consts::PI * (y + 90.0) / 360.0).tan().ln();
            let projected = (HALF_SIZE * x / 180.0, projected_y);
            projected.1.is_finite().then_some(projected).ok_or_else(|| {
                DiscoveryError::Session("invalid WMTS geographic bounding box".into())
            })
        }
    }
}

fn coordinates(text: &str, label: &str) -> Result<(f64, f64), DiscoveryError> {
    let values: Vec<_> = text
        .split_ascii_whitespace()
        .map(str::parse::<f64>)
        .collect::<Result<_, _>>()
        .map_err(|_| DiscoveryError::Session(format!("invalid WMTS {label}")))?;
    match values.as_slice() {
        [x, y] if x.is_finite() && y.is_finite() => Ok((*x, *y)),
        _ => Err(DiscoveryError::Session(format!("invalid WMTS {label}"))),
    }
}

fn positive_number(text: &str, label: &str) -> Result<f64, DiscoveryError> {
    let value = text
        .parse::<f64>()
        .map_err(|_| DiscoveryError::Session(format!("invalid WMTS {label}")))?;
    (value.is_finite() && value > 0.0)
        .then_some(value)
        .ok_or_else(|| DiscoveryError::Session(format!("invalid WMTS {label}")))
}

fn positive_integer(text: &str, label: &str) -> Result<u32, DiscoveryError> {
    let value = positive_number(text, label)?;
    (value.fract() == 0.0 && value <= f64::from(u32::MAX))
        .then(|| value.to_string().parse::<u32>())
        .transpose()
        .map_err(|_| DiscoveryError::Session(format!("invalid WMTS {label}")))?
        .ok_or_else(|| DiscoveryError::Session(format!("invalid WMTS {label}")))
}

fn nonnegative_integer(text: &str, label: &str) -> Result<u32, DiscoveryError> {
    let value = text
        .parse::<u64>()
        .map_err(|_| DiscoveryError::Session(format!("invalid WMTS {label}")))?;
    u32::try_from(value).map_err(|_| DiscoveryError::Session(format!("invalid WMTS {label}")))
}

fn floor_index(value: f64) -> Result<i64, DiscoveryError> {
    let value = value.floor();
    if !value.is_finite() {
        return Err(DiscoveryError::Session(
            "WMTS tile coordinate is out of range".into(),
        ));
    }
    value
        .to_string()
        .parse::<i64>()
        .map_err(|_| DiscoveryError::Session("WMTS tile coordinate is out of range".into()))
}

fn count_between(minimum: u32, maximum: u32) -> Result<u32, DiscoveryError> {
    maximum
        .checked_sub(minimum)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| DiscoveryError::Session("WMTS tile range is too large".into()))
}

fn validate_template(template: &str) -> Result<(), DiscoveryError> {
    let mut remaining = template;
    while let Some(start) = remaining.find('{') {
        let after_start = &remaining[start + 1..];
        let end = after_start.find('}').ok_or_else(|| {
            DiscoveryError::Session("WMTS tile URL template has an unclosed placeholder".into())
        })?;
        let placeholder = &after_start[..end];
        if !is_template_placeholder(placeholder) {
            return Err(DiscoveryError::Session(format!(
                "unsupported WMTS tile URL placeholder: {{{placeholder}}}"
            )));
        }
        remaining = &after_start[end + 1..];
    }
    Ok(())
}

fn is_template_placeholder(value: &str) -> bool {
    ["TileMatrixSet", "TileMatrix", "TileRow", "TileCol", "Style"]
        .iter()
        .any(|known| value.eq_ignore_ascii_case(known))
}

fn render_template(
    template: &str,
    matrix_set: &str,
    matrix: &str,
    style: &str,
    column: u32,
    row: u32,
) -> Request {
    let mut uri = String::with_capacity(template.len() + 32);
    let mut remaining = template;
    while let Some(start) = remaining.find('{') {
        uri.push_str(&remaining[..start]);
        let after_start = &remaining[start + 1..];
        let end = after_start.find('}').unwrap_or(after_start.len());
        let placeholder = &after_start[..end];
        match placeholder {
            value if value.eq_ignore_ascii_case("TileMatrixSet") => uri.push_str(matrix_set),
            value if value.eq_ignore_ascii_case("TileMatrix") => uri.push_str(matrix),
            value if value.eq_ignore_ascii_case("TileRow") => uri.push_str(&row.to_string()),
            value if value.eq_ignore_ascii_case("TileCol") => uri.push_str(&column.to_string()),
            value if value.eq_ignore_ascii_case("Style") => uri.push_str(style),
            _ => {
                uri.push('{');
                uri.push_str(placeholder);
                if end < after_start.len() {
                    uri.push('}');
                }
            }
        }
        remaining = if end < after_start.len() {
            &after_start[end + 1..]
        } else {
            ""
        };
    }
    uri.push_str(remaining);
    Request::new(uri)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CatalogEntry, TileSource};

    #[test]
    fn namespaces_links_limits_and_wgs84_bounds_are_handled_together() {
        let xml = br#"
            <w:Capabilities xmlns:w="http://www.opengis.net/wmts/1.0"
                xmlns:o="http://www.opengis.net/ows/1.1">
              <w:Contents>
                <w:Layer>
                  <o:Identifier>linked-layer</o:Identifier>
                  <o:WGS84BoundingBox>
                    <o:LowerCorner>-10 -10</o:LowerCorner>
                    <o:UpperCorner>10 10</o:UpperCorner>
                  </o:WGS84BoundingBox>
                  <w:Format>image/png</w:Format>
                  <w:Style isDefault="true"><o:Identifier>default-style</o:Identifier></w:Style>
                  <w:TileMatrixSetLink>
                    <w:TileMatrixSet>selected</w:TileMatrixSet>
                    <w:TileMatrixSetLimits>
                      <w:TileMatrixLimits>
                        <o:TileMatrix>0</o:TileMatrix>
                        <w:MinTileRow>1</w:MinTileRow>
                        <w:MaxTileRow>1</w:MaxTileRow>
                        <w:MinTileCol>1</w:MinTileCol>
                        <w:MaxTileCol>1</w:MaxTileCol>
                      </w:TileMatrixLimits>
                    </w:TileMatrixSetLimits>
                  </w:TileMatrixSetLink>
                  <w:ResourceURL format="text/xml" resourceType="FeatureInfo" template="/feature/{TileRow}" />
                  <w:ResourceURL format="image/png" resourceType="tile"
                    template="tiles/{Style}/{TileMatrixSet}/{TileMatrix}/{TileRow}/{TileCol}.png" />
                </w:Layer>
                <w:TileMatrixSet>
                  <o:Identifier>wrong</o:Identifier>
                  <o:SupportedCRS>urn:ogc:def:crs:EPSG::3857</o:SupportedCRS>
                  <w:TileMatrix>
                    <o:Identifier>wrong-matrix</o:Identifier>
                    <w:ScaleDenominator>279541132.0143589</w:ScaleDenominator>
                    <w:TopLeftCorner>-20037508.342789248 20037508.342789248</w:TopLeftCorner>
                    <w:TileWidth>256</w:TileWidth><w:TileHeight>256</w:TileHeight>
                    <w:MatrixWidth>1</w:MatrixWidth><w:MatrixHeight>1</w:MatrixHeight>
                  </w:TileMatrix>
                </w:TileMatrixSet>
                <w:TileMatrixSet>
                  <o:Identifier>selected</o:Identifier>
                  <o:SupportedCRS>urn:ogc:def:crs:EPSG::3857</o:SupportedCRS>
                  <w:TileMatrix>
                    <o:Identifier>0</o:Identifier>
                    <w:ScaleDenominator>279541132.0143589</w:ScaleDenominator>
                    <w:TopLeftCorner>-20037508.342789248 20037508.342789248</w:TopLeftCorner>
                    <w:TileWidth>256</w:TileWidth><w:TileHeight>256</w:TileHeight>
                    <w:MatrixWidth>4</w:MatrixWidth><w:MatrixHeight>4</w:MatrixHeight>
                  </w:TileMatrix>
                </w:TileMatrixSet>
              </w:Contents>
            </w:Capabilities>
        "#;

        let catalog = catalog("https://example.test/wmts/capabilities.xml", xml).unwrap();
        let [CatalogEntry::Ready(image)] = catalog.entries() else {
            panic!("expected one ready image")
        };
        assert_eq!(image.title.as_deref(), Some("linked-layer"));
        let TileSource::Grid(grid) = &image.levels[0].source else {
            panic!("expected a known grid")
        };
        assert_eq!(grid.image_size(), Vec2d::square(256));
        assert_eq!(
            grid.tiles_row_major().next().unwrap().unwrap().request.uri,
            "https://example.test/wmts/tiles/default-style/selected/0/1/1.png"
        );
    }

    #[test]
    fn matrix_dimensions_are_used_without_a_layer_bounding_box() {
        let xml = br#"
            <Capabilities>
              <Contents>
                <Layer>
                  <Identifier>whole-matrix</Identifier>
                  <Format>image/jpeg</Format>
                  <ResourceURL resourceType="tile" format="image/jpeg"
                    template="tiles/{TileMatrixSet}/{TileMatrix}/{TileRow}/{TileCol}.jpg" />
                </Layer>
                <TileMatrixSet>
                  <Identifier>set</Identifier>
                  <SupportedCRS>EPSG:3857</SupportedCRS>
                  <TileMatrix>
                    <Identifier>level</Identifier>
                    <ScaleDenominator>1</ScaleDenominator>
                    <TopLeftCorner>0 0</TopLeftCorner>
                    <TileWidth>128</TileWidth><TileHeight>256</TileHeight>
                    <MatrixWidth>2</MatrixWidth><MatrixHeight>3</MatrixHeight>
                  </TileMatrix>
                </TileMatrixSet>
              </Contents>
            </Capabilities>
        "#;

        let catalog = catalog("https://example.test/wmts/capabilities.xml", xml).unwrap();
        let [CatalogEntry::Ready(image)] = catalog.entries() else {
            panic!("expected one ready image")
        };
        let TileSource::Grid(grid) = &image.levels[0].source else {
            panic!("expected a known grid")
        };
        assert_eq!(grid.image_size(), Vec2d { x: 256, y: 768 });
        assert_eq!(grid.count(), 6);
        assert_eq!(
            grid.tiles_row_major().last().unwrap().unwrap().request.uri,
            "https://example.test/wmts/tiles/set/level/2/1.jpg"
        );
    }

    #[test]
    fn a_non_default_style_is_kept_in_relative_tile_urls() {
        let xml = br#"
            <Capabilities>
              <Contents>
                <Layer>
                  <Identifier>styled</Identifier>
                  <Style isDefault="false"><Identifier>night</Identifier></Style>
                  <ResourceURL resourceType="tile"
                    template="tiles/{Style}/{TileMatrix}/{TileRow}/{TileCol}.jpg" />
                </Layer>
                <TileMatrixSet>
                  <Identifier>set</Identifier>
                  <SupportedCRS>EPSG:3857</SupportedCRS>
                  <TileMatrix>
                    <Identifier>0</Identifier><ScaleDenominator>1</ScaleDenominator>
                    <TopLeftCorner>0 0</TopLeftCorner>
                    <TileWidth>256</TileWidth><TileHeight>256</TileHeight>
                    <MatrixWidth>1</MatrixWidth><MatrixHeight>1</MatrixHeight>
                  </TileMatrix>
                </TileMatrixSet>
              </Contents>
            </Capabilities>
        "#;

        let catalog = catalog("https://example.test/wmts/capabilities.xml", xml).unwrap();
        let [CatalogEntry::Ready(image)] = catalog.entries() else {
            panic!("expected one ready image")
        };
        let TileSource::Grid(grid) = &image.levels[0].source else {
            panic!("expected a known grid")
        };
        assert_eq!(
            grid.tiles_row_major().next().unwrap().unwrap().request.uri,
            "https://example.test/wmts/tiles/night/0/0/0.jpg"
        );
    }

    #[test]
    fn unknown_template_placeholders_are_rejected() {
        let xml = br#"
            <Capabilities>
              <Contents>
                <Layer>
                  <Identifier>invalid-template</Identifier>
                  <ResourceURL resourceType="tile" template="tiles/{Time}/{TileRow}/{TileCol}.jpg" />
                </Layer>
                <TileMatrixSet>
                  <Identifier>set</Identifier>
                  <SupportedCRS>EPSG:3857</SupportedCRS>
                  <TileMatrix>
                    <Identifier>0</Identifier><ScaleDenominator>1</ScaleDenominator>
                    <TopLeftCorner>0 0</TopLeftCorner>
                    <TileWidth>256</TileWidth><TileHeight>256</TileHeight>
                    <MatrixWidth>1</MatrixWidth><MatrixHeight>1</MatrixHeight>
                  </TileMatrix>
                </TileMatrixSet>
              </Contents>
            </Capabilities>
        "#;

        assert!(catalog("https://example.test/wmts/capabilities.xml", xml).is_err());
    }

    #[test]
    fn unknown_bounding_box_crs_is_rejected() {
        let xml = br#"
            <Capabilities>
              <Contents>
                <Layer>
                  <Identifier>unknown-crs</Identifier>
                  <BoundingBox crs="EPSG:3413">
                    <LowerCorner>0 0</LowerCorner>
                    <UpperCorner>1 1</UpperCorner>
                  </BoundingBox>
                  <ResourceURL resourceType="tile"
                    template="tiles/{TileMatrix}/{TileRow}/{TileCol}.jpg" />
                </Layer>
                <TileMatrixSet>
                  <Identifier>set</Identifier>
                  <SupportedCRS>EPSG:3857</SupportedCRS>
                  <TileMatrix>
                    <Identifier>0</Identifier><ScaleDenominator>1</ScaleDenominator>
                    <TopLeftCorner>0 0</TopLeftCorner>
                    <TileWidth>256</TileWidth><TileHeight>256</TileHeight>
                    <MatrixWidth>1</MatrixWidth><MatrixHeight>1</MatrixHeight>
                  </TileMatrix>
                </TileMatrixSet>
              </Contents>
            </Capabilities>
        "#;

        assert!(catalog("https://example.test/wmts/capabilities.xml", xml).is_err());
    }
}
