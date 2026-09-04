use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::{BufReader, Read},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use crate::{MarkerProductScanError, MarkerProfile, MarkerRun, MarkerRunStatus};

pub const MARKER_PRODUCT_INVENTORY_SCHEMA_VERSION: &str = "mathscape.marker-product-inventory.v1";

const STDOUT_LOG: &str = "marker.stdout.log";
const STDERR_LOG: &str = "marker.stderr.log";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarkerProductRole {
    PrimaryMarkdown,
    MarkdownCandidate,
    Metadata,
    Image,
    SupportingFile,
    DiagnosticLog,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct MarkerProductDigest(String);

impl MarkerProductDigest {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerProduct {
    relative_path: PathBuf,
    role: MarkerProductRole,
    media_type: &'static str,
    byte_size: u64,
    digest: MarkerProductDigest,
}

impl MarkerProduct {
    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    pub const fn role(&self) -> MarkerProductRole {
        self.role
    }

    pub const fn media_type(&self) -> &'static str {
        self.media_type
    }

    pub const fn byte_size(&self) -> u64 {
        self.byte_size
    }

    pub const fn digest(&self) -> &MarkerProductDigest {
        &self.digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MarkerProductIssue {
    MissingPrimaryMarkdown { expected: PathBuf },
    AdditionalMarkdown { paths: Vec<PathBuf> },
    MissingMetadata { expected: PathBuf },
    AdditionalMetadata { paths: Vec<PathBuf> },
    UnexpectedOutputLocation { paths: Vec<PathBuf> },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkerProductInventory {
    schema_version: &'static str,
    root: PathBuf,
    output_directory: PathBuf,
    run_status: MarkerRunStatus,
    profile: MarkerProfile,
    primary_markdown: Option<PathBuf>,
    metadata: Option<PathBuf>,
    products: Vec<MarkerProduct>,
    issues: Vec<MarkerProductIssue>,
    total_bytes: u64,
}

impl MarkerProductInventory {
    pub const fn schema_version(&self) -> &'static str {
        self.schema_version
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn output_directory(&self) -> &Path {
        &self.output_directory
    }

    pub const fn run_status(&self) -> MarkerRunStatus {
        self.run_status
    }

    pub const fn profile(&self) -> MarkerProfile {
        self.profile
    }

    pub fn primary_markdown(&self) -> Option<&Path> {
        self.primary_markdown.as_deref()
    }

    pub fn metadata(&self) -> Option<&Path> {
        self.metadata.as_deref()
    }

    pub fn products(&self) -> &[MarkerProduct] {
        &self.products
    }

    pub fn derived_products(&self) -> impl Iterator<Item = &MarkerProduct> {
        self.products
            .iter()
            .filter(|product| product.role != MarkerProductRole::DiagnosticLog)
    }

    pub fn issues(&self) -> &[MarkerProductIssue] {
        &self.issues
    }

    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub fn is_promotable(&self) -> bool {
        self.run_status == MarkerRunStatus::Succeeded
            && self.primary_markdown.is_some()
            && self.metadata.is_some()
            && self.issues.is_empty()
    }

    pub fn source_path(&self, product: &MarkerProduct) -> PathBuf {
        self.root.join(product.relative_path())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MarkerProductScanLimits {
    maximum_files: usize,
    maximum_total_bytes: u64,
    maximum_depth: usize,
}

impl MarkerProductScanLimits {
    pub fn new(
        maximum_files: usize,
        maximum_total_bytes: u64,
        maximum_depth: usize,
    ) -> Result<Self, MarkerProductScanError> {
        if maximum_files == 0 {
            return Err(MarkerProductScanError::InvalidLimit {
                field: "marker.product_scan.maximum_files",
            });
        }
        if maximum_total_bytes == 0 {
            return Err(MarkerProductScanError::InvalidLimit {
                field: "marker.product_scan.maximum_total_bytes",
            });
        }
        if maximum_depth == 0 {
            return Err(MarkerProductScanError::InvalidLimit {
                field: "marker.product_scan.maximum_depth",
            });
        }
        Ok(Self {
            maximum_files,
            maximum_total_bytes,
            maximum_depth,
        })
    }

    pub const fn maximum_files(&self) -> usize {
        self.maximum_files
    }

    pub const fn maximum_total_bytes(&self) -> u64 {
        self.maximum_total_bytes
    }

    pub const fn maximum_depth(&self) -> usize {
        self.maximum_depth
    }
}

impl Default for MarkerProductScanLimits {
    fn default() -> Self {
        Self {
            maximum_files: 10_000,
            maximum_total_bytes: 8 * 1024 * 1024 * 1024,
            maximum_depth: 16,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct MarkerProductScanner {
    limits: MarkerProductScanLimits,
}

impl MarkerProductScanner {
    pub fn new(limits: MarkerProductScanLimits) -> Self {
        Self { limits }
    }

    pub const fn limits(&self) -> MarkerProductScanLimits {
        self.limits
    }

    pub fn scan(&self, run: &MarkerRun) -> Result<MarkerProductInventory, MarkerProductScanError> {
        self.scan_root(
            run.staging().path(),
            run.command().source_pdf(),
            run.status(),
            run.command().profile(),
        )
    }

    fn scan_root(
        &self,
        root: &Path,
        source_pdf: &Path,
        run_status: MarkerRunStatus,
        profile: MarkerProfile,
    ) -> Result<MarkerProductInventory, MarkerProductScanError> {
        validate_root(root)?;
        let base_name = source_pdf
            .file_stem()
            .ok_or_else(|| MarkerProductScanError::MissingSourceStem(source_pdf.to_owned()))?;
        let expected = ExpectedLayout::new(base_name);
        let paths = collect_product_paths(root, self.limits)?;
        let mut total_bytes = 0_u64;
        let mut products = Vec::with_capacity(paths.len());
        for (relative_path, absolute_path, expected_bytes) in paths {
            let (digest, byte_size) = hash_file(&absolute_path)?;
            if byte_size != expected_bytes {
                return Err(MarkerProductScanError::ProductChanged {
                    path: absolute_path,
                    expected_bytes,
                    actual_bytes: byte_size,
                });
            }
            total_bytes = total_bytes.checked_add(byte_size).ok_or(
                MarkerProductScanError::MaximumTotalBytesExceeded {
                    maximum: self.limits.maximum_total_bytes,
                },
            )?;
            if total_bytes > self.limits.maximum_total_bytes {
                return Err(MarkerProductScanError::MaximumTotalBytesExceeded {
                    maximum: self.limits.maximum_total_bytes,
                });
            }
            let role = classify_product(&relative_path, &expected);
            products.push(MarkerProduct {
                media_type: infer_media_type(&relative_path),
                relative_path,
                role,
                byte_size,
                digest,
            });
        }
        products.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));

        let (primary_markdown, metadata, issues) = inspect_expected_layout(&products, &expected);
        Ok(MarkerProductInventory {
            schema_version: MARKER_PRODUCT_INVENTORY_SCHEMA_VERSION,
            root: root.to_owned(),
            output_directory: expected.output_directory,
            run_status,
            profile,
            primary_markdown,
            metadata,
            products,
            issues,
            total_bytes,
        })
    }
}

struct ExpectedLayout {
    output_directory: PathBuf,
    primary_markdown: PathBuf,
    metadata: PathBuf,
}

impl ExpectedLayout {
    fn new(base_name: &OsStr) -> Self {
        let output_directory = PathBuf::from(base_name);
        let primary_markdown = output_directory.join(suffixed_name(base_name, ".md"));
        let metadata = output_directory.join(suffixed_name(base_name, "_meta.json"));
        Self {
            output_directory,
            primary_markdown,
            metadata,
        }
    }
}

fn suffixed_name(base_name: &OsStr, suffix: &str) -> OsString {
    let mut value = base_name.to_os_string();
    value.push(suffix);
    value
}

fn validate_root(root: &Path) -> Result<(), MarkerProductScanError> {
    let metadata = fs::symlink_metadata(root).map_err(|source| MarkerProductScanError::Access {
        path: root.to_owned(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(MarkerProductScanError::SymbolicLink(root.to_owned()));
    }
    if !metadata.is_dir() {
        return Err(MarkerProductScanError::UnsupportedFileType(root.to_owned()));
    }
    Ok(())
}

fn collect_product_paths(
    root: &Path,
    limits: MarkerProductScanLimits,
) -> Result<Vec<(PathBuf, PathBuf, u64)>, MarkerProductScanError> {
    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    let mut directories = vec![(root.to_owned(), 0_usize)];
    while let Some((directory, depth)) = directories.pop() {
        let entries =
            fs::read_dir(&directory).map_err(|source| MarkerProductScanError::Access {
                path: directory.clone(),
                source,
            })?;
        for entry in entries {
            let entry = entry.map_err(|source| MarkerProductScanError::Access {
                path: directory.clone(),
                source,
            })?;
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|source| MarkerProductScanError::Access {
                    path: path.clone(),
                    source,
                })?;
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                return Err(MarkerProductScanError::SymbolicLink(path));
            }
            if file_type.is_dir() {
                let child_depth = depth + 1;
                if child_depth > limits.maximum_depth {
                    return Err(MarkerProductScanError::MaximumDepthExceeded {
                        maximum: limits.maximum_depth,
                        path,
                    });
                }
                directories.push((path, child_depth));
                continue;
            }
            if !file_type.is_file() {
                return Err(MarkerProductScanError::UnsupportedFileType(path));
            }
            let relative_path = path
                .strip_prefix(root)
                .expect("walked entries remain below the run root")
                .to_owned();
            total_bytes = total_bytes.checked_add(metadata.len()).ok_or(
                MarkerProductScanError::MaximumTotalBytesExceeded {
                    maximum: limits.maximum_total_bytes,
                },
            )?;
            if total_bytes > limits.maximum_total_bytes {
                return Err(MarkerProductScanError::MaximumTotalBytesExceeded {
                    maximum: limits.maximum_total_bytes,
                });
            }
            files.push((relative_path, path, metadata.len()));
            if files.len() > limits.maximum_files {
                return Err(MarkerProductScanError::MaximumFileCountExceeded {
                    maximum: limits.maximum_files,
                });
            }
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn hash_file(path: &Path) -> Result<(MarkerProductDigest, u64), MarkerProductScanError> {
    let file = File::open(path).map_err(|source| MarkerProductScanError::Access {
        path: path.to_owned(),
        source,
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut byte_size = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|source| MarkerProductScanError::Access {
                path: path.to_owned(),
                source,
            })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        byte_size = byte_size.saturating_add(count as u64);
    }
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity("sha256:".len() + digest.len() * 2);
    encoded.push_str("sha256:");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok((MarkerProductDigest(encoded), byte_size))
}

fn classify_product(relative_path: &Path, expected: &ExpectedLayout) -> MarkerProductRole {
    if relative_path == Path::new(STDOUT_LOG) || relative_path == Path::new(STDERR_LOG) {
        MarkerProductRole::DiagnosticLog
    } else if relative_path == expected.primary_markdown {
        MarkerProductRole::PrimaryMarkdown
    } else if relative_path == expected.metadata {
        MarkerProductRole::Metadata
    } else if has_extension(relative_path, "md") {
        MarkerProductRole::MarkdownCandidate
    } else if is_image(relative_path) {
        MarkerProductRole::Image
    } else {
        MarkerProductRole::SupportingFile
    }
}

fn inspect_expected_layout(
    products: &[MarkerProduct],
    expected: &ExpectedLayout,
) -> (Option<PathBuf>, Option<PathBuf>, Vec<MarkerProductIssue>) {
    let has_primary = products
        .iter()
        .any(|product| product.relative_path == expected.primary_markdown);
    let has_metadata = products
        .iter()
        .any(|product| product.relative_path == expected.metadata);
    let additional_markdown = products
        .iter()
        .filter(|product| product.role == MarkerProductRole::MarkdownCandidate)
        .map(|product| product.relative_path.clone())
        .collect::<Vec<_>>();
    let additional_metadata = products
        .iter()
        .filter(|product| {
            product.relative_path != expected.metadata
                && product
                    .relative_path
                    .file_name()
                    .and_then(OsStr::to_str)
                    .is_some_and(|name| name.ends_with("_meta.json"))
        })
        .map(|product| product.relative_path.clone())
        .collect::<Vec<_>>();
    let unexpected_locations = products
        .iter()
        .filter(|product| {
            product.role != MarkerProductRole::DiagnosticLog
                && !product
                    .relative_path
                    .starts_with(&expected.output_directory)
        })
        .map(|product| product.relative_path.clone())
        .collect::<Vec<_>>();

    let mut issues = Vec::new();
    if !has_primary {
        issues.push(MarkerProductIssue::MissingPrimaryMarkdown {
            expected: expected.primary_markdown.clone(),
        });
    }
    if !additional_markdown.is_empty() {
        issues.push(MarkerProductIssue::AdditionalMarkdown {
            paths: additional_markdown,
        });
    }
    if !has_metadata {
        issues.push(MarkerProductIssue::MissingMetadata {
            expected: expected.metadata.clone(),
        });
    }
    if !additional_metadata.is_empty() {
        issues.push(MarkerProductIssue::AdditionalMetadata {
            paths: additional_metadata,
        });
    }
    if !unexpected_locations.is_empty() {
        issues.push(MarkerProductIssue::UnexpectedOutputLocation {
            paths: unexpected_locations,
        });
    }
    (
        has_primary.then(|| expected.primary_markdown.clone()),
        has_metadata.then(|| expected.metadata.clone()),
        issues,
    )
}

fn has_extension(path: &Path, expected: &str) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

fn is_image(path: &Path) -> bool {
    [
        "jpg", "jpeg", "png", "gif", "webp", "tif", "tiff", "bmp", "svg",
    ]
    .into_iter()
    .any(|extension| has_extension(path, extension))
}

fn infer_media_type(path: &Path) -> &'static str {
    let Some(extension) = path.extension().and_then(OsStr::to_str) else {
        return "application/octet-stream";
    };
    match extension.to_ascii_lowercase().as_str() {
        "md" => "text/markdown",
        "json" => "application/json",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "tif" | "tiff" => "image/tiff",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "html" | "htm" => "text/html",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_the_committed_real_marker_fixture() {
        let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/marker/marker-analysis-principles-pdf-pages-31-36-v1/run");
        let source_pdf = Path::new("大学数学-数学分析原理与方法_含目录.pdf");
        let inventory = MarkerProductScanner::default()
            .scan_root(
                &fixture_root,
                source_pdf,
                MarkerRunStatus::Succeeded,
                MarkerProfile::MathOcrV1,
            )
            .unwrap();

        assert!(inventory.is_promotable());
        assert!(inventory.issues().is_empty());
        assert_eq!(inventory.products().len(), 2);
        assert_eq!(inventory.derived_products().count(), 2);
        assert_eq!(inventory.total_bytes(), 22_495);

        let markdown = inventory
            .products()
            .iter()
            .find(|product| product.role() == MarkerProductRole::PrimaryMarkdown)
            .unwrap();
        assert_eq!(markdown.byte_size(), 17_444);
        assert_eq!(
            markdown.digest().as_str(),
            "sha256:86e31cb218d5b1d403cb620fd647cf328f1a710d4d02861d8056015749d465fe"
        );

        let metadata = inventory
            .products()
            .iter()
            .find(|product| product.role() == MarkerProductRole::Metadata)
            .unwrap();
        assert_eq!(metadata.byte_size(), 5_051);
        assert_eq!(
            metadata.digest().as_str(),
            "sha256:85c6f2a2bd248049453e045c4efe79b1fa4b3d86a45c84bb71ad1070fd5963ce"
        );

        let metadata_document: serde_json::Value =
            serde_json::from_slice(&fs::read(inventory.source_path(metadata)).unwrap()).unwrap();
        let page_stats = metadata_document["page_stats"].as_array().unwrap();
        let page_ids = page_stats
            .iter()
            .map(|page| page["page_id"].as_u64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(page_ids, vec![30, 31, 32, 33, 34, 35]);
        assert!(
            page_stats
                .iter()
                .all(|page| { page["text_extraction_method"].as_str() == Some("surya") })
        );
    }
}
