; Junction: Groovy (Gradle) highlights; tree-sitter-groovy ships none.
[
  "abstract" "as" "assert" "break" "case" "catch" "class" "continue" "def" "default" "do" "else"
  "enum" "extends" "final" "finally" "for" "if" "implements" "import" "in" "instanceof" "interface"
  "native" "new" "package" "private" "protected" "public" "return" "static" "switch" "synchronized"
  "throw" "throws" "transient" "try" "volatile" "while" "yield"
] @keyword
[(true) (false) (null_literal) (this) (super)] @keyword
[(line_comment) (block_comment)] @comment
(string_literal) @string
(character_literal) @string
(escape_sequence) @string.escape
(string_interpolation) @embedded
[(decimal_integer_literal) (hex_integer_literal) (octal_integer_literal) (binary_integer_literal)
 (decimal_floating_point_literal) (hex_floating_point_literal)] @number
[(integral_type) (floating_point_type) (boolean_type) (void_type)] @keyword
(type_identifier) @type
(annotation name: (_) @attribute)
(marker_annotation) @attribute
(method_invocation name: (identifier) @function)
(juxt_function_call name: (identifier) @function)
(method_declaration name: (identifier) @function)
(function_definition name: (identifier) @function)
(map_item key: (_) @property)
