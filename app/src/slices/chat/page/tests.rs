use super::model_options;

fn ids(options: &[super::ModelOption]) -> Vec<&str> {
    options.iter().map(|option| option.id.as_str()).collect()
}

#[test]
fn favourites_are_separate_from_catalogue_models_without_duplicates() {
    let favourites = vec!["grok-4-mini".to_owned()];
    let listed = vec!["grok-4-mini".to_owned(), "grok-4.6".to_owned()];

    let (favourite_models, catalogue_models, current_favourite) =
        model_options("grok-4.6", &favourites, &listed);

    assert_eq!(ids(&favourite_models), ["grok-4-mini"]);
    assert_eq!(ids(&catalogue_models), ["grok-4.6"]);
    assert!(!favourite_models[0].selected);
    assert!(catalogue_models[0].selected);
    assert!(!current_favourite);
}

#[test]
fn the_current_model_remains_available_without_catalogue_data() {
    let (_, catalogue_models, current_favourite) = model_options("grok-4.6", &[], &[]);

    assert_eq!(ids(&catalogue_models), ["grok-4.6"]);
    assert!(catalogue_models[0].selected);
    assert!(!current_favourite);
}
