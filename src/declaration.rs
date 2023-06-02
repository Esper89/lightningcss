//! CSS declarations.

use std::borrow::Cow;
use std::ops::Range;

use crate::context::PropertyHandlerContext;
use crate::error::{ParserError, PrinterError};
use crate::parser::ParserOptions;
use crate::printer::Printer;
use crate::properties::box_shadow::BoxShadowHandler;
use crate::properties::masking::MaskHandler;
use crate::properties::{
  align::AlignHandler,
  animation::AnimationHandler,
  background::BackgroundHandler,
  border::BorderHandler,
  contain::ContainerHandler,
  display::DisplayHandler,
  flex::FlexHandler,
  font::FontHandler,
  grid::GridHandler,
  list::ListStyleHandler,
  margin_padding::*,
  outline::OutlineHandler,
  overflow::OverflowHandler,
  position::PositionHandler,
  prefix_handler::{FallbackHandler, PrefixHandler},
  size::SizeHandler,
  text::TextDecorationHandler,
  transform::TransformHandler,
  transition::TransitionHandler,
};
use crate::properties::{Property, PropertyId};
use crate::traits::{PropertyHandler, ToCss};
use crate::values::string::CowArcStr;
#[cfg(feature = "visitor")]
use crate::visitor::Visit;
use cssparser::*;

/// A CSS declaration block.
///
/// Properties are separated into a list of `!important` declararations,
/// and a list of normal declarations. This reduces memory usage compared
/// with storing a boolean along with each property.
#[derive(Debug, PartialEq, Clone, Default)]
#[cfg_attr(feature = "visitor", derive(Visit), visit(visit_declaration_block, PROPERTIES))]
#[cfg_attr(feature = "into_owned", derive(lightningcss_derive::IntoOwned))]
#[cfg_attr(
  feature = "serde",
  derive(serde::Serialize, serde::Deserialize),
  serde(rename_all = "camelCase")
)]
#[cfg_attr(feature = "jsonschema", derive(schemars::JsonSchema))]
pub struct DeclarationBlock<'i> {
  /// A list of `!important` declarations in the block.
  #[cfg_attr(feature = "serde", serde(borrow, default))]
  pub important_declarations: Vec<Property<'i>>,
  /// A list of normal declarations in the block.
  #[cfg_attr(feature = "serde", serde(default))]
  pub declarations: Vec<Property<'i>>,
}

impl<'i> DeclarationBlock<'i> {
  /// Parses a declaration block from CSS syntax.
  pub fn parse<'a, 'o, 't>(
    input: &mut Parser<'i, 't>,
    options: &'a ParserOptions<'o, 'i>,
  ) -> Result<Self, ParseError<'i, ParserError<'i>>> {
    let mut important_declarations = DeclarationList::new();
    let mut declarations = DeclarationList::new();
    let mut parser = DeclarationListParser::new(
      input,
      PropertyDeclarationParser {
        important_declarations: &mut important_declarations,
        declarations: &mut declarations,
        options,
      },
    );
    while let Some(res) = parser.next() {
      if let Err((err, _)) = res {
        if options.error_recovery {
          options.warn(err);
          continue;
        }
        return Err(err);
      }
    }

    Ok(DeclarationBlock {
      important_declarations,
      declarations,
    })
  }

  /// Parses a declaration block from a string.
  pub fn parse_string<'o>(
    input: &'i str,
    options: ParserOptions<'o, 'i>,
  ) -> Result<Self, ParseError<'i, ParserError<'i>>> {
    let mut input = ParserInput::new(input);
    let mut parser = Parser::new(&mut input);
    let result = Self::parse(&mut parser, &options)?;
    parser.expect_exhausted()?;
    Ok(result)
  }

  /// Returns an empty declaration block.
  pub fn new() -> Self {
    Self {
      declarations: vec![],
      important_declarations: vec![],
    }
  }

  /// Returns the total number of declarations in the block.
  pub fn len(&self) -> usize {
    self.declarations.len() + self.important_declarations.len()
  }
}

impl<'i> ToCss for DeclarationBlock<'i> {
  fn to_css<W>(&self, dest: &mut Printer<W>) -> Result<(), PrinterError>
  where
    W: std::fmt::Write,
  {
    let len = self.declarations.len() + self.important_declarations.len();
    let mut i = 0;

    macro_rules! write {
      ($decls: expr, $important: literal) => {
        for decl in &$decls {
          decl.to_css(dest, $important)?;
          if i != len - 1 {
            dest.write_char(';')?;
            dest.whitespace()?;
          }
          i += 1;
        }
      };
    }

    write!(self.declarations, false);
    write!(self.important_declarations, true);
    Ok(())
  }
}

impl<'i> DeclarationBlock<'i> {
  /// Writes the declarations to a CSS block, including starting and ending braces.
  pub fn to_css_block<W>(&self, dest: &mut Printer<W>) -> Result<(), PrinterError>
  where
    W: std::fmt::Write,
  {
    dest.whitespace()?;
    dest.write_char('{')?;
    dest.indent();

    let mut i = 0;
    let len = self.len();

    macro_rules! write {
      ($decls: expr, $important: literal) => {
        for decl in &$decls {
          dest.newline()?;
          decl.to_css(dest, $important)?;
          if i != len - 1 || !dest.minify {
            dest.write_char(';')?;
          }
          i += 1;
        }
      };
    }

    write!(self.declarations, false);
    write!(self.important_declarations, true);

    dest.dedent();
    dest.newline()?;
    dest.write_char('}')
  }
}

impl<'i> DeclarationBlock<'i> {
  pub(crate) fn minify(
    &mut self,
    handler: &mut DeclarationHandler<'i>,
    important_handler: &mut DeclarationHandler<'i>,
    context: &mut PropertyHandlerContext<'i, '_>,
  ) {
    macro_rules! handle {
      ($decls: expr, $handler: expr, $important: literal) => {
        for decl in $decls.iter() {
          context.is_important = $important;
          let handled = $handler.handle_property(decl, context);

          if !handled {
            $handler.decls.push(decl.clone());
          }
        }
      };
    }

    handle!(self.important_declarations, important_handler, true);
    handle!(self.declarations, handler, false);

    handler.finalize(context);
    important_handler.finalize(context);
    self.important_declarations = std::mem::take(&mut important_handler.decls);
    self.declarations = std::mem::take(&mut handler.decls);
  }

  /// Returns whether the declaration block is empty.
  pub fn is_empty(&self) -> bool {
    return self.declarations.is_empty() && self.important_declarations.is_empty();
  }

  pub(crate) fn property_location<'t>(
    &self,
    input: &mut Parser<'i, 't>,
    index: usize,
  ) -> Result<(Range<SourceLocation>, Range<SourceLocation>), ParseError<'i, ParserError<'i>>> {
    // Skip to the requested property index.
    for _ in 0..index {
      input.expect_ident()?;
      input.expect_colon()?;
      input.parse_until_after(Delimiter::Semicolon, |parser| {
        while parser.next().is_ok() {}
        Ok(())
      })?;
    }

    // Get property name range.
    input.skip_whitespace();
    let key_start = input.current_source_location();
    input.expect_ident()?;
    let key_end = input.current_source_location();
    let key_range = key_start..key_end;

    input.expect_colon()?;
    input.skip_whitespace();

    // Get value range.
    let val_start = input.current_source_location();
    input.parse_until_before(Delimiter::Semicolon, |parser| {
      while parser.next().is_ok() {}
      Ok(())
    })?;
    let val_end = input.current_source_location();
    let val_range = val_start..val_end;

    Ok((key_range, val_range))
  }
}

impl<'i> DeclarationBlock<'i> {
  /// Returns an iterator over all properties in the declaration.
  pub fn iter(&self) -> impl std::iter::DoubleEndedIterator<Item = (&Property<'i>, bool)> {
    self
      .declarations
      .iter()
      .map(|property| (property, false))
      .chain(self.important_declarations.iter().map(|property| (property, true)))
  }

  /// Returns a mutable iterator over all properties in the declaration.
  pub fn iter_mut(&mut self) -> impl std::iter::DoubleEndedIterator<Item = &mut Property<'i>> {
    self.declarations.iter_mut().chain(self.important_declarations.iter_mut())
  }

  /// Returns the value for a given property id based on the properties in this declaration block.
  ///
  /// If the property is a shorthand, the result will be a combined value of all of the included
  /// longhands, or `None` if some of the longhands are not declared. Otherwise, the value will be
  /// either an explicitly declared longhand, or a value extracted from a shorthand property.
  pub fn get<'a>(&'a self, property_id: &PropertyId) -> Option<(Cow<'a, Property<'i>>, bool)> {
    if property_id.is_shorthand() {
      if let Some((shorthand, important)) = property_id.shorthand_value(&self) {
        return Some((Cow::Owned(shorthand), important));
      }
    } else {
      for (property, important) in self.iter().rev() {
        if property.property_id() == *property_id {
          return Some((Cow::Borrowed(property), important));
        }

        if let Some(val) = property.longhand(&property_id) {
          return Some((Cow::Owned(val), important));
        }
      }
    }

    None
  }

  /// Sets the value and importance for a given property, replacing any existing declarations.
  ///
  /// If the property already exists within the declaration block, it is updated in place. Otherwise,
  /// a new declaration is appended. When updating a longhand property and a shorthand is defined which
  /// includes the longhand, the shorthand will be updated rather than appending a new declaration.
  pub fn set(&mut self, property: Property<'i>, important: bool) {
    let property_id = property.property_id();
    let declarations = if important {
      // Remove any non-important properties with this id.
      self.declarations.retain(|decl| decl.property_id() != property_id);
      &mut self.important_declarations
    } else {
      // Remove any important properties with this id.
      self.important_declarations.retain(|decl| decl.property_id() != property_id);
      &mut self.declarations
    };

    let longhands = property_id.longhands().unwrap_or_else(|| vec![property.property_id()]);

    for decl in declarations.iter_mut().rev() {
      {
        // If any of the longhands being set are in the same logical property group as any of the
        // longhands in this property, but in a different category (i.e. logical or physical),
        // then we cannot modify in place, and need to append a new property.
        let id = decl.property_id();
        let id_longhands = id.longhands().unwrap_or_else(|| vec![id]);
        if longhands.iter().any(|longhand| {
          let logical_group = longhand.logical_group();
          let category = longhand.category();

          logical_group.is_some()
            && id_longhands.iter().any(|id_longhand| {
              logical_group == id_longhand.logical_group() && category != id_longhand.category()
            })
        }) {
          break;
        }
      }

      if decl.property_id() == property_id {
        *decl = property;
        return;
      }

      // Update shorthand.
      if decl.set_longhand(&property).is_ok() {
        return;
      }
    }

    declarations.push(property)
  }

  /// Removes all declarations of the given property id from the declaration block.
  ///
  /// When removing a longhand property and a shorthand is defined which includes the longhand,
  /// the shorthand will be split apart into its component longhand properties, minus the property
  /// to remove. When removing a shorthand, all included longhand properties are also removed.
  pub fn remove(&mut self, property_id: &PropertyId) {
    fn remove<'i, 'a>(declarations: &mut Vec<Property<'i>>, property_id: &PropertyId<'a>) {
      let longhands = property_id.longhands().unwrap_or(vec![]);
      let mut i = 0;
      while i < declarations.len() {
        let replacement = {
          let property = &declarations[i];
          let id = property.property_id();
          if id == *property_id || longhands.contains(&id) {
            // If the property matches the requested property id, or is a longhand
            // property that is included in the requested shorthand, remove it.
            None
          } else if longhands.is_empty() && id.longhands().unwrap_or(vec![]).contains(&property_id) {
            // If this is a shorthand property that includes the requested longhand,
            // split it apart into its component longhands, excluding the requested one.
            Some(
              id.longhands()
                .unwrap()
                .iter()
                .filter_map(|longhand| {
                  if *longhand == *property_id {
                    None
                  } else {
                    property.longhand(longhand)
                  }
                })
                .collect::<Vec<Property>>(),
            )
          } else {
            i += 1;
            continue;
          }
        };

        match replacement {
          Some(properties) => {
            let count = properties.len();
            declarations.splice(i..i + 1, properties);
            i += count;
          }
          None => {
            declarations.remove(i);
          }
        }
      }
    }

    remove(&mut self.declarations, property_id);
    remove(&mut self.important_declarations, property_id);
  }
}

struct PropertyDeclarationParser<'a, 'o, 'i> {
  important_declarations: &'a mut Vec<Property<'i>>,
  declarations: &'a mut Vec<Property<'i>>,
  options: &'a ParserOptions<'o, 'i>,
}

/// Parse a declaration within {} block: `color: blue`
impl<'a, 'o, 'i> cssparser::DeclarationParser<'i> for PropertyDeclarationParser<'a, 'o, 'i> {
  type Declaration = ();
  type Error = ParserError<'i>;

  fn parse_value<'t>(
    &mut self,
    name: CowRcStr<'i>,
    input: &mut cssparser::Parser<'i, 't>,
  ) -> Result<Self::Declaration, cssparser::ParseError<'i, Self::Error>> {
    parse_declaration(
      name,
      input,
      &mut self.declarations,
      &mut self.important_declarations,
      &self.options,
    )
  }
}

/// Default methods reject all at rules.
impl<'a, 'o, 'i> AtRuleParser<'i> for PropertyDeclarationParser<'a, 'o, 'i> {
  type Prelude = ();
  type AtRule = ();
  type Error = ParserError<'i>;
}

pub(crate) fn parse_declaration<'i, 't>(
  name: CowRcStr<'i>,
  input: &mut cssparser::Parser<'i, 't>,
  declarations: &mut DeclarationList<'i>,
  important_declarations: &mut DeclarationList<'i>,
  options: &ParserOptions<'_, 'i>,
) -> Result<(), cssparser::ParseError<'i, ParserError<'i>>> {
  let property = input.parse_until_before(Delimiter::Bang, |input| {
    Property::parse(PropertyId::from(CowArcStr::from(name)), input, options)
  })?;
  let important = input
    .try_parse(|input| {
      input.expect_delim('!')?;
      input.expect_ident_matching("important")
    })
    .is_ok();
  if important {
    important_declarations.push(property);
  } else {
    declarations.push(property);
  }
  Ok(())
}

pub(crate) type DeclarationList<'i> = Vec<Property<'i>>;

#[derive(Default)]
pub(crate) struct DeclarationHandler<'i> {
  background: BackgroundHandler<'i>,
  border: BorderHandler<'i>,
  outline: OutlineHandler,
  flex: FlexHandler,
  grid: GridHandler<'i>,
  align: AlignHandler,
  size: SizeHandler,
  margin: MarginHandler<'i>,
  padding: PaddingHandler<'i>,
  scroll_margin: ScrollMarginHandler<'i>,
  scroll_padding: ScrollPaddingHandler<'i>,
  font: FontHandler<'i>,
  text: TextDecorationHandler<'i>,
  list: ListStyleHandler<'i>,
  transition: TransitionHandler<'i>,
  animation: AnimationHandler<'i>,
  display: DisplayHandler<'i>,
  position: PositionHandler,
  inset: InsetHandler<'i>,
  overflow: OverflowHandler,
  transform: TransformHandler,
  box_shadow: BoxShadowHandler,
  mask: MaskHandler<'i>,
  container: ContainerHandler<'i>,
  fallback: FallbackHandler,
  prefix: PrefixHandler,
  decls: DeclarationList<'i>,
}

impl<'i> DeclarationHandler<'i> {
  pub fn handle_property(
    &mut self,
    property: &Property<'i>,
    context: &mut PropertyHandlerContext<'i, '_>,
  ) -> bool {
    if !context.unused_symbols.is_empty()
      && matches!(property, Property::Custom(custom) if context.unused_symbols.contains(custom.name.as_ref()))
    {
      return true;
    }

    self.background.handle_property(property, &mut self.decls, context)
      || self.border.handle_property(property, &mut self.decls, context)
      || self.outline.handle_property(property, &mut self.decls, context)
      || self.flex.handle_property(property, &mut self.decls, context)
      || self.grid.handle_property(property, &mut self.decls, context)
      || self.align.handle_property(property, &mut self.decls, context)
      || self.size.handle_property(property, &mut self.decls, context)
      || self.margin.handle_property(property, &mut self.decls, context)
      || self.padding.handle_property(property, &mut self.decls, context)
      || self.scroll_margin.handle_property(property, &mut self.decls, context)
      || self.scroll_padding.handle_property(property, &mut self.decls, context)
      || self.font.handle_property(property, &mut self.decls, context)
      || self.text.handle_property(property, &mut self.decls, context)
      || self.list.handle_property(property, &mut self.decls, context)
      || self.transition.handle_property(property, &mut self.decls, context)
      || self.animation.handle_property(property, &mut self.decls, context)
      || self.display.handle_property(property, &mut self.decls, context)
      || self.position.handle_property(property, &mut self.decls, context)
      || self.inset.handle_property(property, &mut self.decls, context)
      || self.overflow.handle_property(property, &mut self.decls, context)
      || self.transform.handle_property(property, &mut self.decls, context)
      || self.box_shadow.handle_property(property, &mut self.decls, context)
      || self.mask.handle_property(property, &mut self.decls, context)
      || self.container.handle_property(property, &mut self.decls, context)
      || self.fallback.handle_property(property, &mut self.decls, context)
      || self.prefix.handle_property(property, &mut self.decls, context)
  }

  pub fn finalize(&mut self, context: &mut PropertyHandlerContext<'i, '_>) {
    self.background.finalize(&mut self.decls, context);
    self.border.finalize(&mut self.decls, context);
    self.outline.finalize(&mut self.decls, context);
    self.flex.finalize(&mut self.decls, context);
    self.grid.finalize(&mut self.decls, context);
    self.align.finalize(&mut self.decls, context);
    self.size.finalize(&mut self.decls, context);
    self.margin.finalize(&mut self.decls, context);
    self.padding.finalize(&mut self.decls, context);
    self.scroll_margin.finalize(&mut self.decls, context);
    self.scroll_padding.finalize(&mut self.decls, context);
    self.font.finalize(&mut self.decls, context);
    self.text.finalize(&mut self.decls, context);
    self.list.finalize(&mut self.decls, context);
    self.transition.finalize(&mut self.decls, context);
    self.animation.finalize(&mut self.decls, context);
    self.display.finalize(&mut self.decls, context);
    self.position.finalize(&mut self.decls, context);
    self.inset.finalize(&mut self.decls, context);
    self.overflow.finalize(&mut self.decls, context);
    self.transform.finalize(&mut self.decls, context);
    self.box_shadow.finalize(&mut self.decls, context);
    self.mask.finalize(&mut self.decls, context);
    self.container.finalize(&mut self.decls, context);
    self.fallback.finalize(&mut self.decls, context);
    self.prefix.finalize(&mut self.decls, context);
  }
}
