
    use super::stand_in_program;

    /// Absent is the shipped daemon: the real agent, by name.
    #[test]
    fn only_an_absent_variable_names_no_stand_in() {
        assert_eq!(stand_in_program(None), None);
    }

    /// Present but empty is a fixture whose path variable was never set. It
    /// meant "not the real one", so it gets `false`.
    #[test]
    fn a_present_but_empty_value_fails_closed() {
        assert_eq!(stand_in_program(Some("")), Some("false".to_string()));
        assert_eq!(stand_in_program(Some("  ")), Some("false".to_string()));
    }

    #[test]
    fn a_plain_absolute_path_is_the_program() {
        assert_eq!(
            stand_in_program(Some("/tmp/fc-x/home/.local/bin/claude")),
            Some("/tmp/fc-x/home/.local/bin/claude".to_string())
        );
    }

    /// Set but unusable fails closed: `false`, never the real agent, and
    /// never a path that would need quoting inside the `-ilc` payload.
    #[test]
    fn anything_else_fails_closed() {
        for raw in ["claude", "bin/claude", "/tmp/with space/claude", "/tmp/x';claude", "/tmp/$HOME"] {
            assert_eq!(stand_in_program(Some(raw)), Some("false".to_string()), "{raw}");
        }
    }
