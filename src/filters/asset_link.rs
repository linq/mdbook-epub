use crate::errors::Error;
use crate::resources::asset::{Asset, AssetKind};
use crate::resources::retrieve::ContentRetriever;
use html_parser::{Dom, Node};
use pulldown_cmark::{CowStr, Event, Tag};
use std::collections::HashMap;
use std::ffi::OsString;
use std::iter;
use std::path::Path;
use url::Url;

/// Filter is used for replacing remote urls with local images downloaded from internet
pub struct AssetRemoteLinkFilter<'a> {
    // Keeps pairs: 'remote url' | 'asset'
    assets: &'a mut HashMap<String, Asset>,
    depth: usize,
    download_handler: &'a dyn ContentRetriever,
}

impl<'a> AssetRemoteLinkFilter<'a> {
    pub(crate) fn new(
        assets: &'a mut HashMap<String, Asset>,
        depth: usize,
        handler: &'a dyn ContentRetriever,
    ) -> Self {
        Self {
            assets,
            depth,
            download_handler: handler,
        }
    }

    /// Do processing of chapter's content and replace 'remote link' by 'local file name'
    pub(crate) fn apply(&mut self, event: Event<'a>) -> Event<'a> {
        debug!("AssetLinkFilter: Processing Event = {:?}", &event);
        match event {
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => self.handle_image_tag(link_type, dest_url, title, id),
            Event::Html(html) | Event::InlineHtml(html) => self.handle_html(html),
            _ => event,
        }
    }

    fn handle_image_tag(
        &mut self,
        link_type: pulldown_cmark::LinkType,
        dest_url: CowStr<'a>,
        title: CowStr<'a>,
        id: CowStr<'a>,
    ) -> Event<'a> {
        let url_str = dest_url.as_ref(); // var shadowing
        if let Some(asset) = self.assets.get_mut(&url_str.to_string()).cloned() {
            debug!("Lookup Remote asset: by {}", &url_str);
            match asset.source {
                AssetKind::Remote(_) => {
                    debug!("Compare: {} vs {}", &asset.original_link, &url_str);
                    // Check equality of remote_url and dest_url
                    if asset.original_link.as_str() == url_str {
                        debug!("1. Found URL '{}' by Event", &url_str);
                        match self.process_asset(&asset, url_str) {
                            Ok(new_file_name) => {
                                debug!("SUCCESSFULLY downloaded resource by URL '{}'", &url_str);
                                let depth = self.depth;
                                let new =
                                    compute_path_prefix(depth, Path::new(new_file_name.as_str()));
                                debug!(
                                    "Create new Event for URL '{}' and new file name = {}",
                                    &url_str, &new
                                );
                                return Event::Start(Tag::Image {
                                    link_type,
                                    dest_url: CowStr::from(new),
                                    title: title.to_owned(),
                                    id: id.to_owned(),
                                });
                            }
                            Err(error) => {
                                error!(
                                    "Can't download resource by URL '{}' for chapter '{}'. Error = {}",
                                    &url_str, &title, error
                                );
                            }
                        }
                    }
                }
                AssetKind::Local(_) => {
                    let depth = self.depth;
                    // local image/resource
                    let new = compute_path_prefix(depth, asset.filename.as_path());
                    return Event::Start(Tag::Image {
                        link_type,
                        dest_url: CowStr::from(new),
                        title: title.to_owned(),
                        id: id.to_owned(),
                    });
                }
            }
        } else {
            error!("No asset found by URL: '{}'", url_str);
        }
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        })
    }

    fn handle_html(&mut self, html: CowStr<'a>) -> Event<'a> {
        let mut found_links = Vec::new();
        if let Ok(dom) = Dom::parse(&html.clone().into_string()) {
            for item in dom.children {
                match item {
                    Node::Element(ref element) if element.name == "img" => {
                        if let Some(dest_url) = &element.attributes["src"] {
                            if Url::parse(dest_url).is_ok() {
                                debug!("Found a valid remote img src:\"{}\".", dest_url);
                                if let Some(asset) = self.assets.get_mut(dest_url).cloned() {
                                    debug!("Lookup Remote asset: by {}", &dest_url);
                                    if let AssetKind::Remote(ref _remote_url) = asset.source {
                                        debug!(
                                            "Compare: {} vs {}",
                                            &asset.original_link, &dest_url
                                        );
                                        // Check equality of remote_url and dest_url
                                        if asset.original_link.as_str() == dest_url.as_str() {
                                            debug!("1. Found URL '{}' by Event", &dest_url);
                                            match self.process_asset(&asset, dest_url) {
                                                Ok(_) => {
                                                    debug!(
                                                        "SUCCESSFULLY downloaded resource by URL '{}'",
                                                        &dest_url
                                                    );
                                                }
                                                Err(error) => {
                                                    error!(
                                                        "Can't download resource by URL '{}'. Error = {}",
                                                        &dest_url, error
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }

                                found_links.push(dest_url.clone());
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if found_links.is_empty() {
            Event::Html(html)
        } else {
            found_links.dedup();
            let mut content = html.clone().into_string();
            debug!("3. found_links\n'{:?}'", &found_links);
            for original_link in found_links {
                // let encoded_link_key = encode_non_ascii_symbols(&original_link);
                debug!("original_link = '{}'", &original_link);
                trace!("1. assets\n'{:?}'", &self.assets);

                if let Some(asset) = self.assets.get(&original_link) {
                    // let new = self.path_prefix(asset.filename.as_path());
                    let depth = self.depth;
                    let new = compute_path_prefix(depth, asset.filename.as_path());

                    trace!("old content before replacement\n{}", &content);
                    trace!(
                        "{:?}, link '{}' is replaced by '{}'",
                        asset, &original_link, &new
                    );
                    // REAL SRC REPLACING happens here...
                    content = content.replace(&original_link, new.as_str());
                    trace!("new content after replacement\n{}", &content);
                } else {
                    error!("Asset was not found by original_link: {}", original_link);
                    unreachable!("{original_link} should be replaced, but it doesn't.");
                }
            }
            Event::Html(CowStr::from(content))
        }
    }

    fn process_asset(
        &mut self,
        asset: &Asset,
        link_key: &str,
        // old_key: &str,
    ) -> Result<String, Error> {
        trace!("1. DUMP assets:\n{:?}\n", self.assets);
        match self.download_handler.download(asset) {
            Ok(updated_data) => {
                let updated_asset = asset.with_updated_fields(updated_data);
                // replaced previous asset by new, updated one
                self.assets
                    .insert(link_key.to_string(), updated_asset.clone());
                trace!("2. DUMP assets:\n{:?}", self.assets);
                Ok(updated_asset.filename.to_string_lossy().to_string())
            }
            Err(error) => Err(error),
        }
    }
}

fn compute_path_prefix(depth: usize, path: &Path) -> String {
    let mut fsp = OsString::new();
    let mut first_component = true;

    for component in path.components() {
        // Skip root directory component for absolute paths
        if matches!(component, std::path::Component::RootDir) {
            continue;
        }

        // Add separator "/" between components (but not before the first one)
        if !first_component {
            fsp.push("/");
        }

        fsp.push(component);
        first_component = false;
    }

    let filename = fsp
        .into_string()
        .unwrap_or_else(|orig| orig.to_string_lossy().to_string());

    (0..depth)
        .map(|_| "..")
        .chain(iter::once(filename.as_str()))
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_compute_path_prefix_zero_depth() {
        let path = Path::new("file.txt");
        assert_eq!(compute_path_prefix(0, path), "file.txt");

        let path = Path::new("dir/file.txt");
        assert_eq!(compute_path_prefix(0, path), "dir/file.txt");
    }

    #[test]
    fn test_compute_path_prefix_with_depth() {
        let path = Path::new("file.txt");
        assert_eq!(compute_path_prefix(1, path), "../file.txt");
        assert_eq!(compute_path_prefix(2, path), "../../file.txt");
        assert_eq!(compute_path_prefix(3, path), "../../../file.txt");
    }

    #[test]
    fn test_compute_path_prefix_with_complex_path() {
        let path = Path::new("dir1/dir2/file.txt");
        assert_eq!(compute_path_prefix(1, path), "../dir1/dir2/file.txt");
        assert_eq!(compute_path_prefix(2, path), "../../dir1/dir2/file.txt");
    }

    #[test]
    fn test_compute_path_prefix_with_absolute_path() {
        let path = Path::new("/dir1/dir2/file.txt");
        assert_eq!(compute_path_prefix(1, path), "../dir1/dir2/file.txt");
        assert_eq!(compute_path_prefix(2, path), "../../dir1/dir2/file.txt");
    }

    #[test]
    fn test_compute_path_prefix_with_empty_path() {
        let path = Path::new("");
        assert_eq!(compute_path_prefix(1, path), "../");
        assert_eq!(compute_path_prefix(2, path), "../../");
    }

    #[test]
    fn test_compute_path_prefix_with_unicode() {
        let path = Path::new("директория/файл.txt");
        assert_eq!(compute_path_prefix(1, path), "../директория/файл.txt");
        assert_eq!(compute_path_prefix(2, path), "../../директория/файл.txt");
    }

    #[test]
    fn test_compute_path_prefix_with_spaces() {
        let path = Path::new("my documents/important file.txt");
        assert_eq!(
            compute_path_prefix(1, path),
            "../my documents/important file.txt"
        );
    }

    #[test]
    fn test_compute_path_prefix_large_depth() {
        let path = Path::new("file.txt");
        let expected = "../../../../../file.txt";
        assert_eq!(compute_path_prefix(5, path), expected);
    }

    #[test]
    fn test_compute_path_prefix_root_only() {
        let path = Path::new("/");
        assert_eq!(compute_path_prefix(1, path), "../");
        assert_eq!(compute_path_prefix(2, path), "../../");
    }

    #[test]
    fn test_compute_path_prefix_dot_paths() {
        let path = Path::new("./file.txt");
        assert_eq!(compute_path_prefix(1, path), ".././file.txt");

        let path = Path::new("../file.txt");
        assert_eq!(compute_path_prefix(1, path), "../../file.txt");
    }
}
