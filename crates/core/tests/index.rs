use tomakast_core::{Action, Index, Item, ItemId, ItemKind};

fn item(provider: &str, local: &str) -> Item {
    Item {
        id: ItemId::new(provider, local),
        title: local.to_string(),
        subtitle: None,
        keywords: vec![],
        icon: None,
        kind: ItemKind::Application,
        actions: vec![Action {
            id: "open".into(),
            title: "Open".into(),
        }],
    }
}

#[test]
fn replace_provider_swaps_only_that_provider() {
    let mut index = Index::new();
    index.replace_provider("apps", vec![item("apps", "a"), item("apps", "b")]);
    index.replace_provider("calc", vec![item("calc", "x")]);
    assert_eq!(index.len(), 3);

    index.replace_provider("apps", vec![item("apps", "c")]);

    assert_eq!(index.len(), 2);
    assert!(index.get(&ItemId::new("apps", "c")).is_some());
    assert!(index.get(&ItemId::new("apps", "a")).is_none());
    assert!(index.get(&ItemId::new("calc", "x")).is_some());
    assert_eq!(index.iter().count(), 2);
}

#[test]
fn item_id_parse_requires_slash() {
    assert!(ItemId::parse("apps/firefox.desktop").is_some());
    assert!(ItemId::parse("firefox").is_none());
    assert_eq!(ItemId::new("apps", "x.desktop").provider(), "apps");
    assert_eq!(ItemId::new("apps", "x.desktop").as_str(), "apps/x.desktop");
}
