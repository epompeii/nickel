use crate::error::{ParseError, ParseErrors};
use crate::files::FileId;
use crate::identifier::LocIdent;
use crate::position::RawSpan;
use crate::term::RichTerm;
use crate::typ::Type;
use lalrpop_util::lalrpop_mod;

lalrpop_mod!(
    #[allow(clippy::all)]
    #[allow(unused_parens)]
    #[allow(unused_imports)]
    pub grammar, "/parser/grammar.rs");

use grammar::__ToTriple;

pub mod error;
pub mod lexer;
pub mod uniterm;
pub mod utils;

#[cfg(test)]
mod tests;

/// Either a term or a toplevel let declaration.
/// Used exclusively in the REPL to allow the defining of variables without having to specify `in`.
/// For instance:
/// ```text
/// nickel>let foo = 1
/// nickel>foo
/// 1
/// ```
pub enum ExtendedTerm {
    RichTerm(RichTerm),
    ToplevelLet(LocIdent, RichTerm),
}

// The interface of LALRPOP-generated parsers, for each public rule. This trait is used as a facade
// to implement parser-independent features (such as error tolerance helpers), which don't have to
// be reimplemented for each and every parser. It's LALRPOP-specific and shouldn't be used outside
// of this module, if we don't want our implementation to be coupled to LALRPOP details.
//
// The type of `parse` was just copy-pasted from the generated code of LALRPOP.
trait LalrpopParser<T> {
    fn parse<'input, 'err, 'wcard, __TOKEN, __TOKENS>(
        &self,
        src_id: FileId,
        errors: &'err mut Vec<
            lalrpop_util::ErrorRecovery<usize, lexer::Token<'input>, self::error::ParseError>,
        >,
        next_wildcard_id: &'wcard mut usize,
        __tokens0: __TOKENS,
    ) -> Result<T, lalrpop_util::ParseError<usize, lexer::Token<'input>, self::error::ParseError>>
    where
        __TOKEN: __ToTriple<'input, 'err, 'wcard>,
        __TOKENS: IntoIterator<Item = __TOKEN>;
}

/// Generate boiler-plate code to implement the trait [`LalrpopParser`] for a parser generated by
/// LALRPOP.
macro_rules! generate_lalrpop_parser_impl {
    ($parser:ty, $output:ty) => {
        impl LalrpopParser<$output> for $parser {
            fn parse<'input, 'err, 'wcard, __TOKEN, __TOKENS>(
                &self,
                src_id: FileId,
                errors: &'err mut Vec<
                    lalrpop_util::ErrorRecovery<
                        usize,
                        lexer::Token<'input>,
                        self::error::ParseError,
                    >,
                >,
                next_wildcard_id: &'wcard mut usize,
                __tokens0: __TOKENS,
            ) -> Result<
                $output,
                lalrpop_util::ParseError<usize, lexer::Token<'input>, self::error::ParseError>,
            >
            where
                __TOKEN: __ToTriple<'input, 'err, 'wcard>,
                __TOKENS: IntoIterator<Item = __TOKEN>,
            {
                Self::parse(self, src_id, errors, next_wildcard_id, __tokens0)
            }
        }
    };
}

generate_lalrpop_parser_impl!(grammar::ExtendedTermParser, ExtendedTerm);
generate_lalrpop_parser_impl!(grammar::TermParser, RichTerm);
generate_lalrpop_parser_impl!(grammar::FixedTypeParser, Type);
generate_lalrpop_parser_impl!(grammar::StaticFieldPathParser, Vec<LocIdent>);
generate_lalrpop_parser_impl!(
    grammar::CliFieldAssignmentParser,
    (Vec<LocIdent>, RichTerm, RawSpan)
);

/// Generic interface of the various specialized Nickel parsers.
///
/// `T` is the product of the parser (a term, a type, etc.).
pub trait ErrorTolerantParser<T> {
    /// Parse a value from a lexer with the given `file_id` in an error-tolerant way. This methods
    /// can still fail for non-recoverable errors.
    fn parse_tolerant(
        &self,
        file_id: FileId,
        lexer: lexer::Lexer,
    ) -> Result<(T, ParseErrors), ParseError>;

    /// Parse a value from a lexer with the given `file_id`, failing at the first encountered
    /// error.
    fn parse_strict(&self, file_id: FileId, lexer: lexer::Lexer) -> Result<T, ParseErrors>;
}

impl<T, P> ErrorTolerantParser<T> for P
where
    P: LalrpopParser<T>,
{
    fn parse_tolerant(
        &self,
        file_id: FileId,
        lexer: lexer::Lexer,
    ) -> Result<(T, ParseErrors), ParseError> {
        let mut parse_errors = Vec::new();
        let mut next_wildcard_id = 0;
        let result = self
            .parse(file_id, &mut parse_errors, &mut next_wildcard_id, lexer)
            .map_err(|err| ParseError::from_lalrpop(err, file_id));

        let parse_errors = ParseErrors::from_recoverable(parse_errors, file_id);
        match result {
            Ok(t) => Ok((t, parse_errors)),
            Err(e) => Err(e),
        }
    }

    fn parse_strict(&self, file_id: FileId, lexer: lexer::Lexer) -> Result<T, ParseErrors> {
        match self.parse_tolerant(file_id, lexer) {
            Ok((t, e)) if e.no_errors() => Ok(t),
            Ok((_, e)) => Err(e),
            Err(e) => Err(e.into()),
        }
    }
}
