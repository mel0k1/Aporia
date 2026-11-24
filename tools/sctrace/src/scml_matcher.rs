use crate::{
    scml_parser::{
        ParserCtx, Pattern, PatternArg, PatternArray, PatternFlagSet, PatternStruct, Patterns,
    },
    strace_parser::{Syscall, SyscallArg, SyscallArray, SyscallFlagSet, SyscallStruct},
};

/// Matcher for syscalls against SCML patterns.
pub(crate) struct Matcher<'a> {
    patterns: Patterns<'a>,
}

impl<'a> Matcher<'a> {
    pub(crate) fn new(patterns: Patterns<'a>) -> Self {
        Self { patterns }
    }

    /// Attempts to match a syscall against available patterns.
    pub(crate) fn match_syscall(&self, syscall: &Syscall) -> Option<&Pattern<'_>> {
        let related_patterns = self.patterns.get(syscall.name())?;

        related_patterns
            .iter()
            .find(|pattern| Self::matches(self.patterns.ctx(), syscall, pattern))
    }

    /// Matches a syscall against a specific pattern.
    fn matches(ctx: &ParserCtx, syscall: &Syscall, pattern: &Pattern) -> bool {
        assert_eq!(syscall.name(), pattern.name());

        if !pattern.wildcard() && syscall.args().len() != pattern.args().len() {
            return false;
        }

        if syscall.args().len() < pattern.args().len() {
            return false;
        }

        for (syscall_arg, pattern_arg) in syscall.args().iter().zip(pattern.args().iter()) {
            if !Self::matches_arg(ctx, syscall_arg, pattern_arg) {
                return false;
            }
        }

        true
    }

    /// Matches a single syscall argument against a pattern argument.
    fn matches_arg(ctx: &ParserCtx, syscall_arg: &SyscallArg, pattern_arg: &PatternArg) -> bool {
        match pattern_arg {
            PatternArg::None => true,

            PatternArg::Integer => matches!(syscall_arg, SyscallArg::Integer(_)),

            PatternArg::Path => todo!("Path matching not implemented yet"),

            PatternArg::Flags(pattern_flags) => match syscall_arg {
                SyscallArg::Integer(value) if *value == "0" => true,

                SyscallArg::Flags(syscall_flags) => {
                    Self::matches_flags(ctx, syscall_flags, pattern_flags)
                }
                _ => false,
            },

            PatternArg::Array(pattern_array) => match syscall_arg {
                SyscallArg::Array(syscall_array) => {
                    Self::matches_array(ctx, syscall_array, pattern_array)
                }

                SyscallArg::Flags(flag_set) if flag_set.flags().len() == 1 => {
                    matches!(
                        flag_set.flags().first(),
                        Some(SyscallArg::Flag(flag_name)) if *flag_name == "NULL"
                    )
                }
                _ => false,
            },

            PatternArg::Struct(pattern_struct) => match syscall_arg {
                SyscallArg::Struct(syscall_struct) => {
                    Self::matches_struct(ctx, syscall_struct, pattern_struct)
                }

                SyscallArg::Flags(flag_set) if flag_set.flags().len() == 1 => {
                    matches!(
                        flag_set.flags().first(),
                        Some(SyscallArg::Flag(flag_name)) if *flag_name == "NULL"
                    )
                }
                _ => false,
            },

            PatternArg::MultipleStruct(pattern_structs) => match syscall_arg {
                SyscallArg::Struct(syscall_struct) => {
                    pattern_structs.structs().iter().any(|pattern_struct| {
                        if let PatternArg::Struct(pattern_struct) = pattern_struct {
                            Self::matches_struct(ctx, syscall_struct, pattern_struct)
                        } else {
                            panic!("Expected PatternArg::Struct inside MultipleStruct");
                        }
                    })
                }

                SyscallArg::Flags(flag_set) if flag_set.flags().len() == 1 => {
                    matches!(
                        flag_set.flags().first(),
                        Some(SyscallArg::Flag(flag_name)) if *flag_name == "NULL"
                    )
                }
                _ => false,
            },

            PatternArg::FlagsVariable(_) | PatternArg::StructVariable(_) => {
                let expanded = pattern_arg.get(ctx);
                Self::matches_arg(ctx, syscall_arg, expanded)
            }

            PatternArg::Flag(_) => {
                panic!(
                    "Single Flag should not appear in pattern matching - should be wrapped in Flags"
                )
            }
        }
    }

    fn matches_flags(
        ctx: &ParserCtx,
        syscall_flags: &SyscallFlagSet,
        pattern_flags: &PatternFlagSet,
    ) -> bool {
        syscall_flags
            .flags()
            .iter()
            .all(|syscall_flag| Self::matches_flag(ctx, syscall_flag, pattern_flags))
    }

    /// Matches a single syscall flag against a pattern flag set.
    fn matches_flag(
        ctx: &ParserCtx,
        syscall_flag: &SyscallArg,
        pattern_flags: &PatternFlagSet,
    ) -> bool {
        match syscall_flag {
            SyscallArg::Flag(_) => (),
            _ => {
                panic!("Syscall flag must be of type Flag for flag matching");
            }
        };

        let mut found_match = false;

        for pattern_flag in pattern_flags.flags() {
            match (syscall_flag, pattern_flag) {
                (SyscallArg::Flag(syscall_name), PatternArg::Flag(pattern_name)) => {
                    if syscall_name == pattern_name {
                        found_match = true;
                        break;
                    }
                }

                (SyscallArg::Integer(_), PatternArg::Integer) => {
                    found_match = true;
                    break;
                }

                (_, PatternArg::FlagsVariable(_)) => {
                    if let PatternArg::Flags(expanded) = pattern_flag.get(ctx) {
                        found_match = Self::matches_flag(ctx, syscall_flag, expanded);
                        if found_match {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }

        found_match
    }

    fn matches_array(
        ctx: &ParserCtx,
        syscall_array: &SyscallArray,
        pattern_array: &PatternArray,
    ) -> bool {
        syscall_array.elements().iter().all(|syscall_element| {
            pattern_array
                .args()
                .iter()
                .any(|pattern_element| Self::matches_arg(ctx, syscall_element, pattern_element))
        })
    }

    fn matches_struct(
        ctx: &ParserCtx,
        syscall_struct: &SyscallStruct,
        pattern_struct: &PatternStruct,
    ) -> bool {
        if !pattern_struct.wildcard()
            && syscall_struct.fields().len() != pattern_struct.fields().len()
        {
            return false;
        }

        for (pattern_field_name, pattern_field_value) in pattern_struct.fields() {
            match syscall_struct.get_value(pattern_field_name) {
                Some(syscall_field_value) => {
                    if !Self::matches_arg(ctx, syscall_field_value, pattern_field_value) {
                        return false;
                    }
                }
                None => {
                    return false;
                }
            }
        }

        true
    }
}
