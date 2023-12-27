use anyhow::Result;
use gtk::{gio, glib, prelude::*, subclass::prelude::*};
use indexmap::{map::Entry, IndexMap};
use serde::{Deserialize, Serialize};

use crate::{recent_item::RecentItem, APP_DATA_DIR};

#[derive(Debug, Serialize, Deserialize)]
struct RecentItemState {
    uri: String,
    added: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    recents: Vec<RecentItemState>,
}

mod imp {
    use std::cell::RefCell;

    use super::*;

    pub struct RecentList {
        pub(super) state_file: gio::File,
        pub(super) list: RefCell<IndexMap<String, RecentItem>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RecentList {
        const NAME: &'static str = "DaggerRecentList";
        type Type = super::RecentList;
        type Interfaces = (gio::ListModel,);

        fn new() -> Self {
            Self {
                state_file: gio::File::for_path(APP_DATA_DIR.join("recents.bin")),
                list: RefCell::new(IndexMap::new()),
            }
        }
    }

    impl ObjectImpl for RecentList {}

    impl ListModelImpl for RecentList {
        fn item_type(&self) -> glib::Type {
            RecentItem::static_type()
        }

        fn n_items(&self) -> u32 {
            self.list.borrow().len() as u32
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.list
                .borrow()
                .get_index(position as usize)
                .map(|(_, v)| v.upcast_ref::<glib::Object>())
                .cloned()
        }
    }
}

glib::wrapper! {
    pub struct RecentList(ObjectSubclass<imp::RecentList>)
        @implements gio::ListModel;
}

impl RecentList {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub async fn load() -> Result<Self> {
        let this = Self::new();
        let imp = this.imp();

        let state = match imp.state_file.load_bytes_future().await {
            Ok((bytes, _)) => bincode::deserialize::<State>(&bytes)?,
            Err(err) => {
                if !err.matches(gio::IOErrorEnum::NotFound) {
                    return Err(err.into());
                }

                State::default()
            }
        };

        let list = state
            .recents
            .iter()
            .map(|recent_state| {
                let uri = &recent_state.uri;
                let item = RecentItem::new(
                    &gio::File::for_uri(uri),
                    &glib::DateTime::from_iso8601(&recent_state.added, None)?,
                );
                anyhow::Ok((uri.to_owned(), item))
            })
            .collect::<Result<IndexMap<_, _>>>()?;
        imp.list.replace(list);

        tracing::debug!(?state, "Recents loaded");

        Ok(this)
    }

    pub async fn save(&self) -> Result<()> {
        let imp = self.imp();

        let recent_states = imp
            .list
            .borrow()
            .iter()
            .map(|(uri, item)| {
                debug_assert_eq!(uri, &item.file().uri());

                RecentItemState {
                    uri: uri.clone(),
                    added: item.added().unwrap().format_iso8601().unwrap().to_string(),
                }
            })
            .collect::<Vec<_>>();

        let state = State {
            recents: recent_states,
        };
        let bytes = bincode::serialize(&state)?;

        imp.state_file
            .replace_contents_future(
                bytes,
                None,
                false,
                gio::FileCreateFlags::REPLACE_DESTINATION,
            )
            .await
            .map_err(|(_, err)| err)?;

        tracing::debug!(?state, "Recents saved");

        Ok(())
    }

    /// Adds the uri to recent items. If it already exists on the list, the
    /// added date time is simply updated to now.
    pub fn add(&self, uri: String) {
        let imp = self.imp();

        let index = match imp.list.borrow_mut().entry(uri) {
            Entry::Vacant(entry) => {
                let uri = entry.key();
                let index = entry.index();

                let item = RecentItem::new(
                    &gio::File::for_uri(uri),
                    &glib::DateTime::now_utc().unwrap(),
                );
                entry.insert(item);

                Some(index)
            }
            Entry::Occupied(entry) => {
                let item = entry.get();
                item.set_added(glib::DateTime::now_utc().unwrap());

                None
            }
        };

        if let Some(index) = index {
            self.items_changed(index as u32, 0, 1);
        }
    }

    pub fn remove(&self, uri: &str) {
        let imp = self.imp();

        let item = imp.list.borrow_mut().shift_remove_full(uri);
        if let Some((position, _, _)) = item {
            self.items_changed(position as u32, 1, 0);
        }
    }
}