use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use indexmap::IndexMap;
use quick_xml::Reader;
use quick_xml::events::Event;
use roxmltree::Document as XmlDocument;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::AppError;
use crate::model::SlideSize;

const CONTENT_TYPES_NS: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const PACKAGE_REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const PRESENTATION_NS: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const OFFICE_REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackagePart {
    bytes: Vec<u8>,
    is_directory: bool,
}

/// A PresentationML package whose original part payloads remain intact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PptxDocument {
    parts: IndexMap<String, PackagePart>,
    slide_count: usize,
    slide_size: SlideSize,
}

impl PptxDocument {
    pub fn new_minimal(size: SlideSize) -> Result<Self, AppError> {
        if !size.is_valid() {
            return Err(AppError::InvalidPptx(
                "slide dimensions must be positive EMU values".to_owned(),
            ));
        }

        let mut parts = IndexMap::new();
        insert_text_part(&mut parts, "[Content_Types].xml", content_types());
        insert_text_part(&mut parts, "_rels/.rels", package_relationships());
        insert_text_part(&mut parts, "ppt/presentation.xml", presentation_xml(size));
        insert_text_part(
            &mut parts,
            "ppt/_rels/presentation.xml.rels",
            presentation_relationships(),
        );
        insert_text_part(
            &mut parts,
            "ppt/slideMasters/slideMaster1.xml",
            slide_master_xml(),
        );
        insert_text_part(
            &mut parts,
            "ppt/slideMasters/_rels/slideMaster1.xml.rels",
            slide_master_relationships(),
        );
        insert_text_part(
            &mut parts,
            "ppt/slideLayouts/slideLayout1.xml",
            slide_layout_xml(),
        );
        insert_text_part(
            &mut parts,
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
            slide_layout_relationships(),
        );
        insert_text_part(&mut parts, "ppt/theme/theme1.xml", theme_xml());
        insert_text_part(&mut parts, "ppt/slides/slide1.xml", slide_xml());
        insert_text_part(
            &mut parts,
            "ppt/slides/_rels/slide1.xml.rels",
            slide_relationships(),
        );

        Self::from_parts(parts)
    }

    pub fn open(path: &Path) -> Result<Self, AppError> {
        let file = File::open(path).map_err(|source| AppError::io(path, source))?;
        Self::from_reader(file)
    }

    pub fn from_reader<R: Read + Seek>(reader: R) -> Result<Self, AppError> {
        let mut archive = ZipArchive::new(reader)?;
        let mut parts = IndexMap::with_capacity(archive.len());

        for index in 0..archive.len() {
            let mut file = archive.by_index(index)?;
            let name = file.name().to_owned();
            let is_directory = file.is_dir();
            validate_part_name(&name, is_directory)?;
            if parts.contains_key(&name) {
                return Err(AppError::InvalidPptx(format!(
                    "duplicate package part `{name}`"
                )));
            }

            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|source| AppError::io(format!("PPTX part {name}"), source))?;
            parts.insert(
                name,
                PackagePart {
                    bytes,
                    is_directory,
                },
            );
        }

        Self::from_parts(parts)
    }

    pub fn slide_count(&self) -> usize {
        self.slide_count
    }

    pub fn slide_size(&self) -> SlideSize {
        self.slide_size
    }

    pub fn part_names(&self) -> impl Iterator<Item = &str> {
        self.parts.keys().map(String::as_str)
    }

    pub fn part_bytes(&self, name: &str) -> Option<&[u8]> {
        self.parts.get(name).map(|part| part.bytes.as_slice())
    }

    pub fn matches_package(&self, other: &Self) -> bool {
        self.slide_count == other.slide_count
            && self.slide_size == other.slide_size
            && self.parts.len() == other.parts.len()
            && self.parts.iter().zip(other.parts.iter()).all(
                |((left_name, left_part), (right_name, right_part))| {
                    left_name == right_name && left_part == right_part
                },
            )
    }

    /// Writes and reopens an in-memory archive, then compares every part before returning bytes.
    pub fn verified_bytes(&self) -> Result<Vec<u8>, AppError> {
        let cursor = self.write_package(std::io::Cursor::new(Vec::new()))?;
        let bytes = cursor.into_inner();
        let reopened = Self::from_reader(std::io::Cursor::new(bytes.as_slice()))?;
        self.ensure_same_parts(&reopened)?;
        Ok(bytes)
    }

    /// Saves beside the destination, verifies the temporary PPTX, then atomically replaces it.
    pub fn save_atomic(&self, path: &Path) -> Result<(), AppError> {
        self.validate()?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let file_name = path
            .file_name()
            .ok_or_else(|| AppError::CommandLine("save path has no filename".to_owned()))?;
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = file_name.to_os_string();
        temporary_name.push(format!(".{}.{}.tmp", std::process::id(), sequence));
        let temporary_path = parent.join(temporary_name);
        let mut guard = TemporaryPackage::new(temporary_path.clone());

        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .map_err(|source| AppError::io(&temporary_path, source))?;
        let file = self.write_package(file)?;
        file.sync_all()
            .map_err(|source| AppError::io(&temporary_path, source))?;

        let saved = Self::open(&temporary_path)?;
        self.ensure_same_parts(&saved)?;
        fs::rename(&temporary_path, path).map_err(|source| AppError::io(path, source))?;
        guard.commit();
        sync_parent_directory(parent);
        Ok(())
    }

    fn from_parts(parts: IndexMap<String, PackagePart>) -> Result<Self, AppError> {
        let document = Self {
            parts,
            slide_count: 0,
            slide_size: SlideSize::WIDESCREEN,
        };
        let slide_count = document.validate()?;
        let slide_size = read_slide_size(&document.parts)?;
        Ok(Self {
            slide_count,
            slide_size,
            ..document
        })
    }

    fn validate(&self) -> Result<usize, AppError> {
        for (name, part) in &self.parts {
            validate_part_name(name, part.is_directory)?;
            if !part.is_directory && is_xml_part(name) {
                validate_xml(name, &part.bytes)?;
            }
        }

        let content_types = parse_part_xml(&self.parts, "[Content_Types].xml")?;
        if !content_types
            .root_element()
            .has_tag_name((CONTENT_TYPES_NS, "Types"))
        {
            return Err(AppError::InvalidPptx(
                "[Content_Types].xml has an unexpected root element".to_owned(),
            ));
        }

        let package_rels = parse_part_xml(&self.parts, "_rels/.rels")?;
        let package_root = package_rels.root_element();
        if !package_root.has_tag_name((PACKAGE_REL_NS, "Relationships")) {
            return Err(AppError::InvalidPptx(
                "_rels/.rels has an unexpected root element".to_owned(),
            ));
        }
        let has_presentation_relationship = package_root.children().any(|relationship| {
            relationship.has_tag_name((PACKAGE_REL_NS, "Relationship"))
                && relationship
                    .attribute("Type")
                    .is_some_and(|kind| kind.ends_with("/officeDocument"))
                && relationship
                    .attribute("Target")
                    .is_some_and(|target| normalize_target(target) == "ppt/presentation.xml")
        });
        if !has_presentation_relationship {
            return Err(AppError::InvalidPptx(
                "root relationships do not point to ppt/presentation.xml".to_owned(),
            ));
        }

        let presentation = parse_part_xml(&self.parts, "ppt/presentation.xml")?;
        if !presentation
            .root_element()
            .has_tag_name((PRESENTATION_NS, "presentation"))
        {
            return Err(AppError::InvalidPptx(
                "ppt/presentation.xml has an unexpected root element".to_owned(),
            ));
        }

        let relationships = parse_part_xml(&self.parts, "ppt/_rels/presentation.xml.rels")?;
        if !relationships
            .root_element()
            .has_tag_name((PACKAGE_REL_NS, "Relationships"))
        {
            return Err(AppError::InvalidPptx(
                "presentation relationships have an unexpected root element".to_owned(),
            ));
        }
        let slide_relationship_ids: HashSet<&str> = relationships
            .root_element()
            .children()
            .filter(|node| node.has_tag_name((PACKAGE_REL_NS, "Relationship")))
            .filter(|node| {
                node.attribute("Type")
                    .is_some_and(|kind| kind.ends_with("/slide"))
            })
            .filter_map(|node| node.attribute("Id"))
            .collect();

        let slide_list = presentation
            .root_element()
            .children()
            .find(|node| node.has_tag_name((PRESENTATION_NS, "sldIdLst")))
            .ok_or_else(|| AppError::InvalidPptx("presentation has no slide list".to_owned()))?;

        let mut slide_count = 0;
        for slide_id in slide_list
            .children()
            .filter(|node| node.has_tag_name((PRESENTATION_NS, "sldId")))
        {
            let relationship_id = slide_id
                .attribute((OFFICE_REL_NS, "id"))
                .ok_or_else(|| AppError::InvalidPptx("slide id has no relationship".to_owned()))?;
            if !slide_relationship_ids.contains(relationship_id) {
                return Err(AppError::InvalidPptx(format!(
                    "slide relationship `{relationship_id}` is missing"
                )));
            }
            slide_count += 1;
        }

        Ok(slide_count)
    }

    fn write_package<W: Write + Seek>(&self, writer: W) -> Result<W, AppError> {
        let mut archive = ZipWriter::new(writer);
        let file_options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o644);
        for (name, part) in &self.parts {
            if part.is_directory {
                archive.add_directory(name, file_options)?;
            } else {
                archive.start_file(name, file_options)?;
                archive
                    .write_all(&part.bytes)
                    .map_err(|source| AppError::io(format!("PPTX part {name}"), source))?;
            }
        }
        archive.finish().map_err(AppError::from)
    }

    fn ensure_same_parts(&self, other: &Self) -> Result<(), AppError> {
        if self.matches_package(other) {
            return Ok(());
        }

        let changed = self
            .parts
            .keys()
            .chain(other.parts.keys())
            .filter(|name| self.parts.get(*name) != other.parts.get(*name))
            .map(String::as_str)
            .collect::<HashSet<_>>();
        Err(AppError::InvalidPptx(format!(
            "save verification found changed package parts: {}",
            changed.into_iter().collect::<Vec<_>>().join(", ")
        )))
    }
}

fn insert_text_part(parts: &mut IndexMap<String, PackagePart>, name: &str, xml: String) {
    parts.insert(
        name.to_owned(),
        PackagePart {
            bytes: xml.into_bytes(),
            is_directory: false,
        },
    );
}

fn parse_part_xml<'a>(
    parts: &'a IndexMap<String, PackagePart>,
    name: &str,
) -> Result<XmlDocument<'a>, AppError> {
    let part = parts
        .get(name)
        .filter(|part| !part.is_directory)
        .ok_or_else(|| {
            AppError::InvalidPptx(format!("required package part `{name}` is missing"))
        })?;
    let text = std::str::from_utf8(&part.bytes).map_err(|error| {
        AppError::InvalidPptx(format!("required XML part `{name}` is not UTF-8: {error}"))
    })?;
    XmlDocument::parse(text).map_err(AppError::from)
}

fn read_slide_size(parts: &IndexMap<String, PackagePart>) -> Result<SlideSize, AppError> {
    let presentation = parse_part_xml(parts, "ppt/presentation.xml")?;
    let dimensions = presentation
        .root_element()
        .children()
        .find(|node| node.has_tag_name((PRESENTATION_NS, "sldSz")))
        .ok_or_else(|| AppError::InvalidPptx("presentation has no p:sldSz element".to_owned()))?;
    let parse_dimension = |name: &str| -> Result<i64, AppError> {
        dimensions
            .attribute(name)
            .and_then(|value| value.parse::<i64>().ok())
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                AppError::InvalidPptx(format!("presentation p:sldSz has invalid {name}"))
            })
    };
    Ok(SlideSize {
        width: crate::model::Emu(parse_dimension("cx")?),
        height: crate::model::Emu(parse_dimension("cy")?),
    })
}

fn validate_xml(name: &str, bytes: &[u8]) -> Result<(), AppError> {
    let mut reader = Reader::from_reader(bytes);
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Eof) => return Ok(()),
            Ok(_) => buffer.clear(),
            Err(source) => {
                return Err(AppError::Xml {
                    part: name.to_owned(),
                    source,
                });
            }
        }
    }
}

fn is_xml_part(name: &str) -> bool {
    name.ends_with(".xml") || name.ends_with(".rels")
}

fn validate_part_name(name: &str, is_directory: bool) -> Result<(), AppError> {
    let normalized = if is_directory {
        name.strip_suffix('/').unwrap_or(name)
    } else {
        name
    };
    let invalid = normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains('\\')
        || normalized.contains(':')
        || normalized.contains('\0')
        || normalized
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..");
    if invalid {
        return Err(AppError::InvalidPptx(format!(
            "unsafe package part name `{name}`"
        )));
    }
    if is_directory && !name.ends_with('/') {
        return Err(AppError::InvalidPptx(format!(
            "directory entry `{name}` has no trailing slash"
        )));
    }
    Ok(())
}

fn normalize_target(target: &str) -> &str {
    target.strip_prefix('/').unwrap_or(target)
}

struct TemporaryPackage {
    path: PathBuf,
    committed: bool,
}

impl TemporaryPackage {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TemporaryPackage {
    fn drop(&mut self) {
        if !self.committed
            && let Err(error) = fs::remove_file(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %self.path.display(), error = %error, "Could not remove incomplete temporary PPTX");
        }
    }
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) {
    if let Err(error) = File::open(parent).and_then(|directory| directory.sync_all()) {
        tracing::warn!(path = %parent.display(), error = %error, "Could not sync PPTX destination directory");
    }
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) {}

fn content_types() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/ppt/presentation.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml"/>
  <Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/>
  <Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml"/>
  <Override PartName="/ppt/slides/slide1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slide+xml"/>
  <Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>
</Types>
"#
    .to_owned()
}

fn package_relationships() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>
</Relationships>
"#
    .to_owned()
}

fn presentation_xml(size: SlideSize) -> String {
    let slide_type = if size.width.0.checked_mul(3) == size.height.0.checked_mul(4) {
        "screen4x3"
    } else if size.width.0.checked_mul(9) == size.height.0.checked_mul(16) {
        "screen16x9"
    } else {
        "custom"
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:presentation xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>
  <p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst>
  <p:sldSz cx="{}" cy="{}" type="{}"/>
  <p:notesSz cx="6858000" cy="9144000"/>
  <p:defaultTextStyle><a:defPPr><a:defRPr lang="en-US"/></a:defPPr></p:defaultTextStyle>
</p:presentation>
"#,
        size.width.0, size.height.0, slide_type
    )
}

fn presentation_relationships() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="slideMasters/slideMaster1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide" Target="slides/slide1.xml"/>
</Relationships>
"#
    .to_owned()
}

fn slide_master_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldMaster xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld name="Blank"><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld>
  <p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>
  <p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst>
  <p:txStyles><p:titleStyle/><p:bodyStyle/><p:otherStyle/></p:txStyles>
</p:sldMaster>
"#
    .to_owned()
}

fn slide_master_relationships() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme" Target="../theme/theme1.xml"/>
</Relationships>
"#
    .to_owned()
}

fn slide_layout_xml() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sldLayout xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" type="blank" preserve="1">
  <p:cSld name="Blank">{}</p:cSld>
  <p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>
</p:sldLayout>
"#,
        empty_shape_tree()
    )
}

fn slide_layout_relationships() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster" Target="../slideMasters/slideMaster1.xml"/>
</Relationships>
"#
    .to_owned()
}

fn slide_xml() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" showMasterSp="1" showMasterPhAnim="1">
  <p:cSld name="Slide 1">{}</p:cSld>
  <p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr>
</p:sld>
"#,
        empty_shape_tree()
    )
}

fn empty_shape_tree() -> String {
    r#"<p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr></p:spTree>"#.to_owned()
}

fn slide_relationships() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout" Target="../slideLayouts/slideLayout1.xml"/>
</Relationships>
"#
    .to_owned()
}

fn theme_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme">
  <a:themeElements>
    <a:clrScheme name="Office">
      <a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1>
      <a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
      <a:dk2><a:srgbClr val="44546A"/></a:dk2>
      <a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
      <a:accent1><a:srgbClr val="4472C4"/></a:accent1>
      <a:accent2><a:srgbClr val="ED7D31"/></a:accent2>
      <a:accent3><a:srgbClr val="A5A5A5"/></a:accent3>
      <a:accent4><a:srgbClr val="FFC000"/></a:accent4>
      <a:accent5><a:srgbClr val="5B9BD5"/></a:accent5>
      <a:accent6><a:srgbClr val="70AD47"/></a:accent6>
      <a:hlink><a:srgbClr val="0563C1"/></a:hlink>
      <a:folHlink><a:srgbClr val="954F72"/></a:folHlink>
    </a:clrScheme>
    <a:fontScheme name="Office"><a:majorFont><a:latin typeface="Caladea"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Carlito"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme>
    <a:fmtScheme name="Office">
      <a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst>
      <a:lnStyleLst><a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln><a:ln w="12700"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln><a:ln w="19050"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:prstDash val="solid"/></a:ln></a:lnStyleLst>
      <a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst>
      <a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst>
    </a:fmtScheme>
  </a:themeElements>
</a:theme>
"#
    .to_owned()
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use indexmap::IndexMap;

    use super::{PackagePart, PptxDocument};
    use crate::model::SlideSize;

    fn make_package_with_extensions() -> Result<PptxDocument, crate::AppError> {
        let base = PptxDocument::new_minimal(SlideSize::WIDESCREEN)?;
        let mut parts = base.parts.clone();
        let slide = parts
            .get_mut("ppt/slides/slide1.xml")
            .ok_or_else(|| crate::AppError::InvalidPptx("generated slide is missing".to_owned()))?;
        let xml = String::from_utf8(slide.bytes.clone()).map_err(|error| {
            crate::AppError::InvalidPptx(format!("generated XML is not UTF-8: {error}"))
        })?;
        let extension_xml = xml.replace(
            "</p:sld>",
            "<p:extLst><p:ext uri=\"{11111111-1111-1111-1111-111111111111}\"><mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice Requires=\"p\"><p:extensionData value=\"keep exactly\"/></mc:Choice><mc:Fallback/></mc:AlternateContent></p:ext></p:extLst></p:sld>",
        );
        slide.bytes = extension_xml.into_bytes();
        parts.insert(
            "customXml/item1.xml".to_owned(),
            PackagePart {
                bytes:
                    b"<?xml version=\"1.0\"?><sample vendor=\"unknown\">preserve payload</sample>"
                        .to_vec(),
                is_directory: false,
            },
        );
        PptxDocument::from_parts(parts)
    }

    #[test]
    fn blank_widescreen_and_classic_decks_are_valid_ooxml() {
        let wide = PptxDocument::new_minimal(SlideSize::WIDESCREEN);
        let classic = PptxDocument::new_minimal(SlideSize::CLASSIC_4_3);
        assert!(
            wide.as_ref()
                .is_ok_and(|document| document.slide_size() == SlideSize::WIDESCREEN)
        );
        assert!(
            classic
                .as_ref()
                .is_ok_and(|document| document.slide_size() == SlideSize::CLASSIC_4_3)
        );
        if let Ok(document) = wide {
            assert_eq!(document.slide_count(), 1);
            assert!(
                document
                    .part_names()
                    .any(|name| name == "ppt/theme/theme1.xml")
            );
        }
    }

    #[test]
    fn verified_round_trip_preserves_unknown_parts_and_extension_xml_bytes() {
        let package = make_package_with_extensions();
        assert!(package.is_ok());
        if let Ok(document) = package {
            let original_parts: IndexMap<String, PackagePart> = document.parts.clone();
            let output = document.verified_bytes();
            assert!(output.is_ok());
            if let Ok(bytes) = output {
                let reopened = PptxDocument::from_reader(Cursor::new(bytes));
                assert!(reopened.is_ok());
                if let Ok(reopened) = reopened {
                    assert_eq!(reopened.parts, original_parts);
                    assert_eq!(reopened.slide_count(), 1);
                    let slide = reopened.part_bytes("ppt/slides/slide1.xml");
                    assert!(slide.is_some_and(|payload| {
                        let signature = b"<mc:AlternateContent";
                        payload
                            .windows(signature.len())
                            .any(|window| window == signature)
                    }));
                    assert!(reopened.part_bytes("customXml/item1.xml").is_some());
                }
            }
        }
    }

    #[test]
    fn rejects_unsafe_zip_member_names() {
        let mut parts = IndexMap::new();
        parts.insert(
            "../outside.xml".to_owned(),
            PackagePart {
                bytes: Vec::new(),
                is_directory: false,
            },
        );
        let result = PptxDocument::from_parts(parts);
        assert!(result.is_err());
    }
}
