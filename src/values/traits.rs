use cssparser::*;
use crate::properties::Property;

pub trait Parse: Sized {
  /// Parse a value of this type.
  ///
  /// Returns an error on failure.
  fn parse<'i, 't>(
      input: &mut Parser<'i, 't>,
  ) -> Result<Self, ParseError<'i, ()>>;
}

/// Trait for things the can serialize themselves in CSS syntax.
pub trait ToCss {
  /// Serialize `self` in CSS syntax, writing to `dest`.
  fn to_css<W>(&self, dest: &mut W) -> std::fmt::Result where W: std::fmt::Write;

  /// Serialize `self` in CSS syntax and return a string.
  ///
  /// (This is a convenience wrapper for `to_css` and probably should not be overridden.)
  #[inline]
  fn to_css_string(&self) -> String {
      let mut s = String::new();
      self.to_css(&mut s).unwrap();
      s
  }
}

impl<'a, T> ToCss for &'a T
where
    T: ToCss + ?Sized,
{
    fn to_css<W>(&self, dest: &mut W) -> std::fmt::Result where W: std::fmt::Write {
      (*self).to_css(dest)
    }
}

pub trait PropertyHandler: Sized {
  fn handle_property(&mut self, property: &Property) -> bool;
  fn finalize(&mut self) -> Vec<Property>;
}