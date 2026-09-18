//! Configuration controls do not contact a catalog or initialize rendering.
use provlita::config::{Config, MAX_CONFIG_BYTES, Outputs};
const EXAMPLE: &str = include_str!("../examples/minimal/config.kdl");
#[test]
fn example_has_three_pins_and_a_sixty_four_pixel_strip() {
    let config = Config::parse(EXAMPLE).unwrap();
    assert_eq!(config.outputs, Outputs::All);
    assert_eq!(
        config
            .pins
            .iter()
            .map(|p| p.identity.as_str())
            .collect::<Vec<_>>(),
        [
            "registered:terminal",
            "registered:browser",
            "registered:files"
        ]
    );
    assert_eq!(config.logical_size(), (172, 64));
}
#[test]
fn malformed_unbounded_or_command_configuration_refuses() {
    for bad in [
        EXAMPLE.replace("schema 1", "schema 2"),
        EXAMPLE.replace("tile-size 48", "tile-size 0"),
        EXAMPLE.replace("spacing 6", "spacing 100"),
        EXAMPLE.replace("outputs \"all\"", "outputs 1 1"),
        EXAMPLE.replace("outputs \"all\"", "outputs 0"),
        EXAMPLE.replace("outputs \"all\"", "outputs \"all\" 1"),
        EXAMPLE.replace("edge \"bottom\"", "edge \"top\""),
        EXAMPLE.replace("align \"center\"", "align \"left\""),
        EXAMPLE.replace("icon=\"terminal\"", "icon=\"/home/user/icon.png\""),
        EXAMPLE.replace("pin \"registered:files\"", "pin \"registered:terminal\""),
        EXAMPLE.replace("registered:terminal", "terminal"),
        EXAMPLE.replace("registered:terminal", "registered:"),
        EXAMPLE.replace("spacing 6", "exec \"sh\""),
        EXAMPLE.replace("spacing 6", "spacing 6\nspacing 7"),
        "x".repeat(MAX_CONFIG_BYTES + 1),
    ] {
        assert!(Config::parse(&bad).is_err(), "{bad}");
    }
}
#[test]
fn output_selection_and_pin_limits_are_explicit() {
    assert_eq!(
        Config::parse(&EXAMPLE.replace("outputs \"all\"", "outputs 2 7"))
            .unwrap()
            .outputs,
        Outputs::Selected(vec![2, 7])
    );
    let pins = (0..17)
        .map(|i| format!("pin \"registered:id{i}\" label=\"app\" icon=\"files\";\n"))
        .collect::<String>();
    assert!(Config::parse(&format!("schema 1\ndock {{ {pins} }}")).is_err());
}
