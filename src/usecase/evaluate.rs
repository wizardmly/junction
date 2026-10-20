//! Evaluate Expression, Watches and breakpoint conditions, as IntelliJ's
//! Java evaluator runs them on a paused thread: names are the frame's
//! locals, then `this`'s fields, then the class's statics, then classes;
//! fields fall back to Kotlin getters; methods are found up the class
//! chain and called on the thread; arithmetic promotes and unboxes as Java
//! does, and `+` with a string calls `toString()`; `new` calls a
//! constructor (so does a Kotlin-style `Point(1, 2)`), casts convert or
//! check the class, and assignments set locals, fields and elements. Also
//! how the Variables
//! view describes an object: `size = 3` for a collection, its `toString()`
//! when its class has one.

use crate::domain::debugger::{value_text, ClassRef, DebugVm, Frame, MethodInfo, Value, Variable};
use crate::domain::expression::{parse, BinaryOp, Expr, UnaryOp, PRIMITIVES};

/// Where an expression is evaluated: a paused thread's frame.
pub struct Scope<'a> {
    pub vm: &'a dyn DebugVm,
    pub thread: u64,
    pub frame: &'a Frame,
    /// The frame's `this` and locals.
    pub locals: &'a [Variable],
}

/// What a name or a dotted path means.
enum Target {
    Value(Value),
    Class(ClassRef),
}

/// Parses and evaluates an expression.
pub fn evaluate(scope: &Scope, text: &str) -> Result<Value, String> {
    let expr = parse(text)?;
    scope.eval(&expr)
}

/// A breakpoint condition: whether to stop.
pub fn condition_holds(scope: &Scope, text: &str) -> Result<bool, String> {
    match scope.unbox(evaluate(scope, text)?)? {
        Value::Boolean(b) => Ok(b),
        other => Err(format!("Boolean value expected, got '{}'", value_text(&other))),
    }
}

/// The wrapper classes and their primitives' JNI types.
const BOXES: &[(&str, &str)] = &[
    ("java.lang.Integer", "I"),
    ("java.lang.Long", "J"),
    ("java.lang.Double", "D"),
    ("java.lang.Float", "F"),
    ("java.lang.Short", "S"),
    ("java.lang.Byte", "B"),
    ("java.lang.Character", "C"),
    ("java.lang.Boolean", "Z"),
];

/// A method signature's parameter types: `(IJLjava/lang/String;[I)V` →
/// `I`, `J`, `Ljava/lang/String;`, `[I`.
fn params(signature: &str) -> Vec<String> {
    let inner = signature.strip_prefix('(').and_then(|s| s.split(')').next()).unwrap_or("");
    let chars: Vec<char> = inner.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        while chars[i] == '[' {
            i += 1;
        }
        if chars[i] == 'L' {
            while chars[i] != ';' {
                i += 1;
            }
        }
        i += 1;
        out.push(chars[start..i].iter().collect());
    }
    out
}

fn is_numeric(v: &Value) -> bool {
    matches!(v, Value::Byte(_) | Value::Short(_) | Value::Char(_) | Value::Int(_) | Value::Long(_) | Value::Float(_) | Value::Double(_))
}

fn as_i64(v: &Value) -> Option<i64> {
    Some(match v {
        Value::Byte(n) => *n as i64,
        Value::Short(n) => *n as i64,
        Value::Char(c) => *c as i64,
        Value::Int(n) => *n as i64,
        Value::Long(n) => *n,
        _ => return None,
    })
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Float(f) => Some(*f as f64),
        Value::Double(f) => Some(*f),
        other => as_i64(other).map(|n| n as f64),
    }
}

/// A value converted to a primitive parameter type, if it fits.
fn convert(value: &Value, ty: &str) -> Option<Value> {
    Some(match (ty, value) {
        ("Z", Value::Boolean(b)) => Value::Boolean(*b),
        ("I", v) if !matches!(v, Value::Long(_) | Value::Float(_) | Value::Double(_)) => Value::Int(as_i64(v)? as i32),
        ("J", v) if !matches!(v, Value::Float(_) | Value::Double(_)) => Value::Long(as_i64(v)?),
        ("D", v) => Value::Double(as_f64(v)?),
        ("F", v) if !matches!(v, Value::Double(_)) => Value::Float(as_f64(v)? as f32),
        ("S", Value::Short(n)) => Value::Short(*n),
        ("B", Value::Byte(n)) => Value::Byte(*n),
        ("C", Value::Char(c)) => Value::Char(*c),
        _ => return None,
    })
}

/// Of overloads taking as many arguments, the one whose parameters best
/// fit them: primitives for primitives (boxes too), references for
/// objects.
fn best_overload(methods: Vec<MethodInfo>, args: &[Value]) -> Option<MethodInfo> {
    let fit = |m: &MethodInfo| -> i32 {
        params(&m.signature)
            .iter()
            .zip(args)
            .map(|(ty, arg)| {
                let reference = ty.starts_with('L') || ty.starts_with('[');
                let boxed = matches!(arg, Value::Object { class, .. } if BOXES.iter().any(|(w, _)| w == class));
                let primitive = is_numeric(arg) || matches!(arg, Value::Boolean(_));
                let letter = match arg {
                    Value::Int(_) => "I",
                    Value::Long(_) => "J",
                    Value::Double(_) => "D",
                    Value::Float(_) => "F",
                    Value::Short(_) => "S",
                    Value::Byte(_) => "B",
                    Value::Char(_) => "C",
                    Value::Boolean(_) => "Z",
                    _ => "",
                };
                match (reference, primitive || boxed) {
                    // The exact type first, then a widening.
                    (false, true) if letter == ty => 3,
                    (false, true) if primitive && convert(arg, ty).is_none() => -100,
                    (false, true) => 2,
                    (true, false) if ty == "Ljava/lang/String;" && !matches!(arg, Value::String { .. } | Value::Null) => 0,
                    (true, false) => 2,
                    (true, true) => 1,
                    (false, false) => -100,
                }
            })
            .sum()
    };
    // The first of equally fitting ones.
    methods.into_iter().map(|m| (fit(&m), m)).fold(None, |best: Option<(i32, MethodInfo)>, (score, m)| match best {
        Some((s, _)) if s >= score => best,
        _ => Some((score, m)),
    }).map(|(_, m)| m)
}

/// A primitive cast, as Java's narrowing and widening conversions.
fn cast_primitive(value: &Value, ty: &str) -> Option<Value> {
    if let Value::Boolean(b) = value {
        return (ty == "Z").then_some(Value::Boolean(*b));
    }
    if ty == "Z" {
        return None;
    }
    let whole = as_i64(value);
    let real = as_f64(value)?;
    // Java truncates toward zero; NaN is 0, out of range saturates.
    let truncated = whole.unwrap_or(if real.is_nan() { 0 } else { real as i64 });
    Some(match ty {
        "I" => Value::Int(whole.map(|n| n as i32).unwrap_or(if real.is_nan() { 0 } else { real as i32 })),
        "J" => Value::Long(truncated),
        "S" => Value::Short(truncated as i16),
        "B" => Value::Byte(truncated as i8),
        "C" => Value::Char(truncated as u16),
        "F" => Value::Float(real as f32),
        "D" => Value::Double(real),
        _ => return None,
    })
}

/// A JNI type as Java writes it: `I` → `int`, `Ljava/lang/String;` →
/// `java.lang.String`.
fn type_name(ty: &str) -> String {
    match PRIMITIVES.iter().find(|(_, jni)| *jni == ty) {
        Some((name, _)) => name.to_string(),
        None => crate::domain::debugger::java_name(ty),
    }
}

impl Scope<'_> {
    fn this(&self) -> Option<&Value> {
        self.locals.iter().find(|v| v.name == "this").map(|v| &v.value)
    }

    fn eval(&self, expr: &Expr) -> Result<Value, String> {
        match self.target(expr)? {
            Target::Value(v) => Ok(v),
            Target::Class(c) => Err(format!("'{}' is a class, not a value", c.name)),
        }
    }

    /// A dotted name (`java.util.Collections`), for class lookups.
    fn dotted(expr: &Expr) -> Option<String> {
        match expr {
            Expr::Name(n) => Some(n.clone()),
            Expr::Field(t, n) => Some(format!("{}.{n}", Self::dotted(t)?)),
            _ => None,
        }
    }

    /// A class by a simple or qualified name: as written, in `java.lang`,
    /// or in the frame's package.
    fn class_named(&self, name: &str) -> Result<Option<ClassRef>, String> {
        let package = self.frame.class.rsplit_once('.').map(|(p, _)| p.to_owned());
        let mut candidates = vec![name.to_owned()];
        if !name.contains('.') {
            candidates.push(format!("java.lang.{name}"));
            // A class nested in the frame's (or its outer) class.
            let mut outer = self.frame.class.as_str();
            loop {
                candidates.push(format!("{outer}${name}"));
                match outer.rsplit_once('$') {
                    Some((o, _)) => outer = o,
                    None => break,
                }
            }
        }
        if let Some(p) = package {
            candidates.push(format!("{p}.{name}"));
        }
        for candidate in candidates {
            // `Outer.Inner` is `Outer$Inner`: try the last dots as `$`.
            let mut name = candidate;
            for _ in 0..3 {
                if let Some(class) = self.vm.find_class(&name)? {
                    return Ok(Some(class));
                }
                let Some(dot) = name.rfind('.') else { break };
                name.replace_range(dot..dot + 1, "$");
            }
        }
        Ok(None)
    }

    fn target(&self, expr: &Expr) -> Result<Target, String> {
        Ok(Target::Value(match expr {
            Expr::Int(n) => Value::Int(*n as i32),
            Expr::Long(n) => Value::Long(*n),
            Expr::Double(f) => Value::Double(*f),
            Expr::Bool(b) => Value::Boolean(*b),
            Expr::Char(c) => Value::Char(*c as u16),
            Expr::Null => Value::Null,
            Expr::Str(s) => self.vm.create_string(s)?,
            Expr::This => self.this().cloned().ok_or("'this' is not available")?,
            Expr::Name(name) => {
                if let Some(v) = self.locals.iter().find(|v| v.name == *name) {
                    return Ok(Target::Value(v.value.clone()));
                }
                if let Some(this) = self.this().and_then(Value::object_id) {
                    let class = self.vm.class_of(this)?;
                    if let Some(v) = self.vm.field(class.id, Some(this), name)? {
                        return Ok(Target::Value(v));
                    }
                }
                if let Some(v) = self.vm.field(self.frame.location.class, None, name)? {
                    return Ok(Target::Value(v));
                }
                if let Some(class) = self.class_named(name)? {
                    return Ok(Target::Class(class));
                }
                return Err(format!("Cannot find local variable '{name}'"));
            }
            Expr::Field(target, name) => {
                let target = match self.target(target) {
                    Ok(t) => t,
                    // `java.util.Collections`: a package path.
                    Err(e) => match Self::dotted(expr).map(|d| self.class_named(&d)).transpose()?.flatten() {
                        Some(class) => return Ok(Target::Class(class)),
                        None => return Err(e),
                    },
                };
                match target {
                    Target::Class(class) => match self.vm.field(class.id, None, name)? {
                        Some(v) => v,
                        // A nested class: `Outer.Inner` is `Outer$Inner`.
                        None => match self.class_named(&format!("{}${name}", class.name))? {
                            Some(nested) => return Ok(Target::Class(nested)),
                            None => return Err(format!("Cannot find field '{name}' in {}", class.name)),
                        },
                    },
                    Target::Value(value) => self.member(&value, name)?,
                }
            }
            Expr::Index(target, index) => {
                let target = self.eval(target)?;
                let index = self.unbox(self.eval(index)?)?;
                let ix = as_i64(&index).ok_or("Array index must be an integer")?;
                match &target {
                    Value::Array { id, .. } => self.vm.array_element(*id, ix.max(0) as usize)?,
                    Value::Null => return Err("java.lang.NullPointerException".into()),
                    // A list: `list[0]` as Kotlin writes `get(0)`.
                    _ => self.call_on(&target, "get", vec![Value::Int(ix as i32)])?,
                }
            }
            Expr::Call(target, name, args) => {
                let args = args.iter().map(|a| self.eval(a)).collect::<Result<Vec<_>, _>>()?;
                match target {
                    Some(t) => match self.target(t) {
                        Ok(Target::Value(v)) => self.call_on(&v, name, args)?,
                        Ok(Target::Class(c)) => self.call_static(&c, name, args)?,
                        Err(e) => match Self::dotted(t).map(|d| self.class_named(&d)).transpose()?.flatten() {
                            Some(c) => self.call_static(&c, name, args)?,
                            None => return Err(e),
                        },
                    },
                    None => {
                        let called = match self.this().cloned().filter(|t| t.object_id().is_some()) {
                            Some(this) => self.call_on(&this, name, args.clone()),
                            None => {
                                let class = ClassRef { id: self.frame.location.class, name: self.frame.class.clone() };
                                self.call_static(&class, name, args.clone())
                            }
                        };
                        match called {
                            // Kotlin creates objects without `new`.
                            Err(e) if e.starts_with("Cannot find") && name.starts_with(char::is_uppercase) => match self.class_named(name)? {
                                Some(class) => self.construct(&class, args)?,
                                None => return Err(e),
                            },
                            other => other?,
                        }
                    }
                }
            }
            Expr::Unary(op, e) => {
                let v = self.unbox(self.eval(e)?)?;
                match (op, &v) {
                    (UnaryOp::Not, Value::Boolean(b)) => Value::Boolean(!b),
                    (UnaryOp::Neg, Value::Double(f)) => Value::Double(-f),
                    (UnaryOp::Neg, Value::Float(f)) => Value::Float(-f),
                    (UnaryOp::Neg, Value::Long(n)) => Value::Long(n.wrapping_neg()),
                    (UnaryOp::Neg, v) if is_numeric(v) => Value::Int((as_i64(v).unwrap_or(0) as i32).wrapping_neg()),
                    _ => return Err(format!("Operator cannot be applied to '{}'", value_text(&v))),
                }
            }
            Expr::Binary(op, a, b) => self.binary(*op, a, b)?,
            Expr::New(ty, args) => {
                let class = self.class_named(ty)?.ok_or_else(|| format!("Cannot resolve class '{ty}'"))?;
                let args = args.iter().map(|a| self.eval(a)).collect::<Result<Vec<_>, _>>()?;
                self.construct(&class, args)?
            }
            Expr::Cast(ty, e) => {
                let value = self.eval(e)?;
                self.cast(value, ty)?
            }
            Expr::InstanceOf(e, ty) => {
                let value = self.eval(e)?;
                Value::Boolean(self.is_instance(&value, ty)?)
            }
            Expr::Assign(target, value) => {
                let value = self.eval(value)?;
                self.assign(target, value)?
            }
        }))
    }

    /// `new`: the class's constructor taking the arguments.
    fn construct(&self, class: &ClassRef, args: Vec<Value>) -> Result<Value, String> {
        let constructors: Vec<MethodInfo> = self.vm.methods(class.id)?.into_iter().filter(|m| m.name == "<init>" && params(&m.signature).len() == args.len()).collect();
        let Some(constructor) = best_overload(constructors, &args) else {
            return Err(format!("Cannot find constructor of {} with {} arguments", class.name, args.len()));
        };
        let args = self.arguments(&constructor, args)?;
        self.vm.new_instance(self.thread, class.id, constructor.id, &args)
    }

    /// Whether a class is, extends or implements another.
    fn assignable(&self, class: u64, to: u64) -> Result<bool, String> {
        let mut pending = vec![class];
        let mut seen = std::collections::HashSet::new();
        while let Some(at) = pending.pop() {
            if at == to {
                return Ok(true);
            }
            if at == 0 || !seen.insert(at) || seen.len() > 256 {
                continue;
            }
            pending.push(self.vm.superclass(at)?);
            pending.extend(self.vm.interfaces(at)?);
        }
        Ok(false)
    }

    /// `instanceof`: false for null; a primitive is its box.
    fn is_instance(&self, value: &Value, ty: &str) -> Result<bool, String> {
        let value = match value {
            Value::Null => return Ok(false),
            v if is_numeric(v) || matches!(v, Value::Boolean(_)) => self.box_value(v.clone())?,
            v => v.clone(),
        };
        let (Value::String { id, .. } | Value::Object { id, .. } | Value::Array { id, .. }) = value else { return Ok(false) };
        let target = self.class_named(ty)?.ok_or_else(|| format!("Cannot resolve class '{ty}'"))?;
        if target.name == "java.lang.Object" {
            return Ok(true);
        }
        let class = self.vm.class_of(id)?;
        self.assignable(class.id, target.id)
    }

    /// A cast: a primitive converts as Java's (truncating, wrapping); a
    /// reference must be an instance of the type.
    fn cast(&self, value: Value, ty: &str) -> Result<Value, String> {
        if let Some((_, jni)) = PRIMITIVES.iter().find(|(name, _)| *name == ty) {
            let value = self.unbox(value)?;
            return cast_primitive(&value, jni).ok_or_else(|| format!("Inconvertible types; cannot cast '{}' to '{ty}'", value_text(&value)));
        }
        if matches!(value, Value::Null) {
            return Ok(value);
        }
        if self.is_instance(&value, ty)? {
            let boxed = is_numeric(&value) || matches!(value, Value::Boolean(_));
            return if boxed { self.box_value(value) } else { Ok(value) };
        }
        let class = match &value {
            Value::String { .. } => "java.lang.String".to_owned(),
            Value::Object { class, .. } | Value::Array { class, .. } => class.clone(),
            other => value_text(other),
        };
        Err(format!("java.lang.ClassCastException: class {class} cannot be cast to class {ty}"))
    }

    /// A value as a variable of a JNI type holds it: a primitive
    /// converted, boxed for a reference.
    fn coerce(&self, value: Value, ty: &str) -> Result<Value, String> {
        if ty.starts_with('L') || ty.starts_with('[') {
            return match value {
                v if is_numeric(&v) || matches!(v, Value::Boolean(_)) => {
                    // A primitive goes into its box, or a supertype of it.
                    let class = crate::domain::debugger::java_name(ty);
                    let boxable = BOXES.iter().any(|(w, _)| *w == class) || matches!(class.as_str(), "java.lang.Object" | "java.lang.Number" | "java.lang.Comparable" | "java.io.Serializable");
                    if !boxable {
                        return Err(format!("Incompatible types: '{}' cannot be assigned to '{class}'", value_text(&v)));
                    }
                    self.box_value(v)
                }
                v => Ok(v),
            };
        }
        let value = self.unbox(value)?;
        convert(&value, ty).ok_or_else(|| format!("Incompatible types: '{}' cannot be assigned to '{}'", value_text(&value), type_name(ty)))
    }

    /// `target = value`: a local, a field of `this` or a static of the
    /// frame's class; `x.f`; `a[i]`. The value assigned.
    fn assign(&self, target: &Expr, value: Value) -> Result<Value, String> {
        match target {
            Expr::Name(name) => {
                if let Some(ty) = self.vm.variable_type(self.thread, self.frame, name)? {
                    let value = self.coerce(value, &ty)?;
                    self.vm.set_variable(self.thread, self.frame, name, &value)?;
                    return Ok(value);
                }
                if let Some(this) = self.this().and_then(Value::object_id) {
                    let class = self.vm.class_of(this)?;
                    if let Some(ty) = self.vm.field_type(class.id, name)? {
                        let value = self.coerce(value, &ty)?;
                        self.vm.set_field(class.id, Some(this), name, &value)?;
                        return Ok(value);
                    }
                }
                let class = self.frame.location.class;
                match self.vm.field_type(class, name)? {
                    Some(ty) => {
                        let value = self.coerce(value, &ty)?;
                        self.vm.set_field(class, None, name, &value)?;
                        Ok(value)
                    }
                    None => Err(format!("Cannot find local variable '{name}'")),
                }
            }
            Expr::Field(owner, name) => {
                let (class, object) = match self.target(owner)? {
                    Target::Class(class) => (class.id, None),
                    Target::Value(Value::Null) => return Err("java.lang.NullPointerException".into()),
                    Target::Value(v) => {
                        let id = match &v {
                            Value::Object { id, .. } | Value::String { id, .. } | Value::Array { id, .. } => *id,
                            other => return Err(format!("Cannot find field '{name}' in {}", value_text(other))),
                        };
                        (self.vm.class_of(id)?.id, Some(id))
                    }
                };
                let ty = self.vm.field_type(class, name)?.ok_or_else(|| format!("Cannot find field '{name}'"))?;
                let value = self.coerce(value, &ty)?;
                self.vm.set_field(class, object, name, &value)?;
                Ok(value)
            }
            Expr::Index(array, index) => {
                let array = self.eval(array)?;
                let ix = as_i64(&self.unbox(self.eval(index)?)?).ok_or("Array index must be an integer")?;
                let Value::Array { id, class, length } = array else { return Err("Array expected".into()) };
                if ix < 0 || ix as usize >= length {
                    return Err(format!("java.lang.ArrayIndexOutOfBoundsException: Index {ix} out of bounds for length {length}"));
                }
                let component = class.strip_suffix("[]").unwrap_or(&class);
                let ty = match PRIMITIVES.iter().find(|(name, _)| *name == component) {
                    Some((_, jni)) => jni.to_string(),
                    None => format!("L{};", component.replace('.', "/")),
                };
                let value = self.coerce(value, &ty)?;
                self.vm.set_array_element(id, ix as usize, &value)?;
                Ok(value)
            }
            _ => Err("Variable expected".into()),
        }
    }

    /// `value.name`: an array's length, a field, a Kotlin property's
    /// getter, `size()` / `length()`.
    fn member(&self, value: &Value, name: &str) -> Result<Value, String> {
        match value {
            Value::Null => return Err("java.lang.NullPointerException".into()),
            Value::Array { length, .. } if name == "length" || name == "size" => return Ok(Value::Int(*length as i32)),
            _ => {}
        }
        let object = value.object_id().or(match value {
            Value::String { id, .. } => Some(*id),
            _ => None,
        });
        let Some(object) = object else { return Err(format!("Cannot find field '{name}'")) };
        let class = self.vm.class_of(object)?;
        if let Some(v) = self.vm.field(class.id, Some(object), name)? {
            return Ok(v);
        }
        let mut cap = name.to_owned();
        if let Some(first) = cap.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        for getter in [format!("get{cap}"), format!("is{cap}"), name.to_owned()] {
            if self.find_method(class.id, &getter, &[], false)?.is_some() {
                return self.call_on(value, &getter, Vec::new());
            }
        }
        Err(format!("Cannot find field '{name}' in {}", class.name))
    }

    /// A method by name and argument count, up the class chain: (its
    /// class, the method).
    fn find_method(&self, class: u64, name: &str, args: &[Value], want_static: bool) -> Result<Option<(u64, MethodInfo)>, String> {
        let mut at = class;
        let mut depth = 0;
        while at != 0 && depth < 32 {
            depth += 1;
            let overloads: Vec<MethodInfo> = self.vm.methods(at)?.into_iter().filter(|m| m.name == name && params(&m.signature).len() == args.len() && (!want_static || m.is_static)).collect();
            if let Some(m) = best_overload(overloads, args) {
                return Ok(Some((at, m)));
            }
            at = self.vm.superclass(at)?;
        }
        Ok(None)
    }

    /// Arguments as a method's parameters take them: widened, boxed.
    fn arguments(&self, method: &MethodInfo, args: Vec<Value>) -> Result<Vec<Value>, String> {
        params(&method.signature)
            .iter()
            .zip(args)
            .map(|(ty, arg)| {
                if ty.starts_with('L') || ty.starts_with('[') {
                    match arg {
                        v if is_numeric(&v) || matches!(v, Value::Boolean(_)) => self.box_value(v),
                        v => Ok(v),
                    }
                } else {
                    let arg = self.unbox(arg)?;
                    convert(&arg, ty).ok_or_else(|| format!("Incompatible argument '{}' for {}", value_text(&arg), method.name))
                }
            })
            .collect()
    }

    fn call_on(&self, target: &Value, name: &str, args: Vec<Value>) -> Result<Value, String> {
        let object = match target {
            Value::Null => return Err("java.lang.NullPointerException".into()),
            Value::String { id, .. } | Value::Object { id, .. } | Value::Array { id, .. } => *id,
            // A primitive's method: on its box (`x.toString()`).
            v => return self.call_on(&self.box_value(v.clone())?, name, args),
        };
        let class = self.vm.class_of(object)?;
        let Some((owner, method)) = self.find_method(class.id, name, &args, false)? else {
            return Err(format!("Cannot find method '{name}' in {}", class.name));
        };
        let args = self.arguments(&method, args)?;
        let object = (!method.is_static).then_some(object);
        self.vm.invoke(self.thread, owner, object, method.id, &args)
    }

    fn call_static(&self, class: &ClassRef, name: &str, args: Vec<Value>) -> Result<Value, String> {
        let Some((owner, method)) = self.find_method(class.id, name, &args, true)? else {
            return Err(format!("Cannot find static method '{name}' in {}", class.name));
        };
        let args = self.arguments(&method, args)?;
        self.vm.invoke(self.thread, owner, None, method.id, &args)
    }

    /// A primitive in its wrapper (`Integer.valueOf`).
    fn box_value(&self, value: Value) -> Result<Value, String> {
        let ty = match value {
            Value::Int(_) => "I",
            Value::Long(_) => "J",
            Value::Double(_) => "D",
            Value::Float(_) => "F",
            Value::Short(_) => "S",
            Value::Byte(_) => "B",
            Value::Char(_) => "C",
            Value::Boolean(_) => "Z",
            other => return Ok(other),
        };
        let (wrapper, _) = BOXES.iter().find(|(_, t)| *t == ty).unwrap();
        let class = self.vm.find_class(wrapper)?.ok_or_else(|| format!("{wrapper} is not loaded"))?;
        let signature = format!("({ty})L{};", wrapper.replace('.', "/"));
        let method = self.vm.methods(class.id)?.into_iter().find(|m| m.name == "valueOf" && m.signature == signature).ok_or("valueOf not found")?;
        self.vm.invoke(self.thread, class.id, None, method.id, &[value])
    }

    /// A wrapper's primitive (`Integer` → `int`); other values as they are.
    fn unbox(&self, value: Value) -> Result<Value, String> {
        if let Value::Object { id, class } = &value {
            if BOXES.iter().any(|(w, _)| w == class) {
                let class_id = self.vm.class_of(*id)?.id;
                if let Some(v) = self.vm.field(class_id, Some(*id), "value")? {
                    return Ok(v);
                }
            }
        }
        Ok(value)
    }

    /// A value's text as `+` with a string writes it.
    fn text_of(&self, value: &Value) -> Result<String, String> {
        Ok(match value {
            Value::String { text, .. } => text.clone(),
            Value::Char(c) => char::from_u32(*c as u32).map(String::from).unwrap_or_default(),
            Value::Null => "null".into(),
            Value::Object { .. } | Value::Array { .. } => match self.call_on(value, "toString", Vec::new())? {
                Value::String { text, .. } => text,
                other => value_text(&other),
            },
            other => value_text(other),
        })
    }

    fn binary(&self, op: BinaryOp, a: &Expr, b: &Expr) -> Result<Value, String> {
        // Short-circuit.
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            let Value::Boolean(left) = self.unbox(self.eval(a)?)? else { return Err("Boolean value expected".into()) };
            if left == (op == BinaryOp::Or) {
                return Ok(Value::Boolean(left));
            }
            let Value::Boolean(right) = self.unbox(self.eval(b)?)? else { return Err("Boolean value expected".into()) };
            return Ok(Value::Boolean(right));
        }
        let (left, right) = (self.eval(a)?, self.eval(b)?);
        if op == BinaryOp::Add && (matches!(left, Value::String { .. }) || matches!(right, Value::String { .. })) {
            let text = format!("{}{}", self.text_of(&left)?, self.text_of(&right)?);
            return self.vm.create_string(&text);
        }
        // Identity of objects, as Java's ==.
        if matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
            let id = |v: &Value| match v {
                Value::Null => Some(0),
                Value::String { id, .. } | Value::Object { id, .. } | Value::Array { id, .. } => Some(*id),
                _ => None,
            };
            let boxed = |v: &Value| matches!(v, Value::Object { class, .. } if BOXES.iter().any(|(w, _)| w == class));
            if let (Some(x), Some(y)) = (id(&left), id(&right)) {
                if !(boxed(&left) && boxed(&right)) || x == y {
                    let same = x == y;
                    return Ok(Value::Boolean(if op == BinaryOp::Eq { same } else { !same }));
                }
            }
        }
        let (left, right) = (self.unbox(left)?, self.unbox(right)?);
        if let (Value::Boolean(x), Value::Boolean(y)) = (&left, &right) {
            return match op {
                BinaryOp::Eq => Ok(Value::Boolean(x == y)),
                BinaryOp::Ne => Ok(Value::Boolean(x != y)),
                _ => Err("Operator cannot be applied to boolean".into()),
            };
        }
        if !is_numeric(&left) || !is_numeric(&right) {
            return Err(format!("Operator cannot be applied to '{}', '{}'", value_text(&left), value_text(&right)));
        }
        let floating = matches!(left, Value::Float(_) | Value::Double(_)) || matches!(right, Value::Float(_) | Value::Double(_));
        if floating {
            let (x, y) = (as_f64(&left).unwrap(), as_f64(&right).unwrap());
            let single = !matches!(left, Value::Double(_)) && !matches!(right, Value::Double(_));
            let number = |f: f64| if single { Value::Float(f as f32) } else { Value::Double(f) };
            return Ok(match op {
                BinaryOp::Add => number(x + y),
                BinaryOp::Sub => number(x - y),
                BinaryOp::Mul => number(x * y),
                BinaryOp::Div => number(x / y),
                BinaryOp::Rem => number(x % y),
                cmp => Value::Boolean(compare(cmp, x.partial_cmp(&y))),
            });
        }
        let (x, y) = (as_i64(&left).unwrap(), as_i64(&right).unwrap());
        let long = matches!(left, Value::Long(_)) || matches!(right, Value::Long(_));
        if matches!(op, BinaryOp::Div | BinaryOp::Rem) && y == 0 {
            return Err("java.lang.ArithmeticException: / by zero".into());
        }
        let number = |n: i64| if long { Value::Long(n) } else { Value::Int(n as i32) };
        let (xi, yi) = (x as i32, y as i32);
        Ok(match op {
            BinaryOp::Add => if long { number(x.wrapping_add(y)) } else { number(xi.wrapping_add(yi) as i64) },
            BinaryOp::Sub => if long { number(x.wrapping_sub(y)) } else { number(xi.wrapping_sub(yi) as i64) },
            BinaryOp::Mul => if long { number(x.wrapping_mul(y)) } else { number(xi.wrapping_mul(yi) as i64) },
            BinaryOp::Div => if long { number(x.wrapping_div(y)) } else { number(xi.wrapping_div(yi) as i64) },
            BinaryOp::Rem => if long { number(x.wrapping_rem(y)) } else { number(xi.wrapping_rem(yi) as i64) },
            cmp => Value::Boolean(compare(cmp, x.partial_cmp(&y))),
        })
    }
}

fn compare(op: BinaryOp, ordering: Option<std::cmp::Ordering>) -> bool {
    use std::cmp::Ordering::*;
    match (op, ordering) {
        (_, None) => op == BinaryOp::Ne,
        (BinaryOp::Eq, Some(o)) => o == Equal,
        (BinaryOp::Ne, Some(o)) => o != Equal,
        (BinaryOp::Lt, Some(o)) => o == Less,
        (BinaryOp::Le, Some(o)) => o != Greater,
        (BinaryOp::Gt, Some(o)) => o == Greater,
        (BinaryOp::Ge, Some(o)) => o != Less,
        _ => false,
    }
}

// --- Describing values -------------------------------------------------------

/// The class chain's names, the class first.
fn chain(vm: &dyn DebugVm, class: u64) -> Result<Vec<(u64, String)>, String> {
    let mut out = Vec::new();
    let mut at = class;
    while at != 0 && out.len() < 32 {
        let name = vm.class_name(at)?;
        out.push((at, name));
        at = vm.superclass(at)?;
    }
    Ok(out)
}

/// What the Variables view writes after an object's `{Class@id}`: a
/// java.util collection's `size = 3`, a boxed number's value, else its
/// `toString()` in quotes when its class (not `Object`) declares one.
pub fn describe(vm: &dyn DebugVm, thread: u64, value: &Value) -> Option<String> {
    let Value::Object { id, class } = value else { return None };
    let class_id = vm.class_of(*id).ok()?.id;
    if BOXES.iter().any(|(w, _)| w == class) {
        return vm.field(class_id, Some(*id), "value").ok().flatten().map(|v| value_text(&v));
    }
    let names = chain(vm, class_id).ok()?;
    let invoke0 = |name: &str, signature: &str| -> Option<Value> {
        for (at, owner) in &names {
            if owner == "java.lang.Object" {
                return None;
            }
            if let Some(m) = vm.methods(*at).ok()?.into_iter().find(|m| m.name == name && m.signature == signature && !m.is_static) {
                return vm.invoke(thread, *at, Some(*id), m.id, &[]).ok();
            }
        }
        None
    };
    if names.iter().any(|(_, n)| n.starts_with("java.util.")) {
        if let Some(Value::Int(size)) = invoke0("size", "()I") {
            return Some(format!("size = {size}"));
        }
    }
    match invoke0("toString", "()Ljava/lang/String;") {
        Some(Value::String { text, .. }) => {
            let short: String = text.chars().take(100).collect();
            Some(format!("\"{short}{}\"", if short.len() < text.len() { "…" } else { "" }))
        }
        _ => None,
    }
}

/// A value as text, as IntelliJ's Evaluate and log prints it: a string's
/// own text, a char itself, an object's `toString()`.
pub fn as_string(vm: &dyn DebugVm, thread: u64, value: &Value) -> Result<String, String> {
    let id = match value {
        Value::String { text, .. } => return Ok(text.clone()),
        Value::Char(c) => return Ok(char::from_u32(*c as u32).map(String::from).unwrap_or_default()),
        Value::Object { id, .. } | Value::Array { id, .. } => *id,
        other => return Ok(value_text(other)),
    };
    for (at, _) in chain(vm, vm.class_of(id)?.id)? {
        if let Some(m) = vm.methods(at)?.into_iter().find(|m| m.name == "toString" && m.signature == "()Ljava/lang/String;" && !m.is_static) {
            return match vm.invoke(thread, at, Some(id), m.id, &[])? {
                Value::String { text, .. } => Ok(text),
                other => Ok(value_text(&other)),
            };
        }
    }
    Ok(value_text(value))
}

/// An expanded value's children: a java.util collection's elements (its
/// `toArray()`, a map's entries), else its fields or an array's elements.
pub fn children(vm: &dyn DebugVm, thread: u64, value: &Value, limit: usize) -> Result<Vec<Variable>, String> {
    let Some(object) = value.object_id() else { return Ok(Vec::new()) };
    if let Value::Object { .. } = value {
        let class = vm.class_of(object)?;
        let names = chain(vm, class.id)?;
        if names.iter().any(|(_, n)| n.starts_with("java.util.")) {
            let find = |name: &str, signature: &str| -> Result<Option<(u64, u64)>, String> {
                for (at, _) in &names {
                    if let Some(m) = vm.methods(*at)?.into_iter().find(|m| m.name == name && m.signature == signature && !m.is_static) {
                        return Ok(Some((*at, m.id)));
                    }
                }
                Ok(None)
            };
            let elements = if let Some((owner, method)) = find("toArray", "()[Ljava/lang/Object;")? {
                Some(vm.invoke(thread, owner, Some(object), method, &[])?)
            } else if let Some((owner, method)) = find("entrySet", "()Ljava/util/Set;")? {
                let set = vm.invoke(thread, owner, Some(object), method, &[])?;
                Some(Scope::array_of(vm, thread, &set)?)
            } else {
                None
            };
            if let Some(Value::Array { id, .. }) = elements {
                return vm.children(id, limit);
            }
        }
    }
    vm.children(object, limit)
}

impl Scope<'_> {
    /// A collection's `toArray()`.
    fn array_of(vm: &dyn DebugVm, thread: u64, collection: &Value) -> Result<Value, String> {
        let Some(object) = collection.object_id() else { return Ok(Value::Null) };
        let class = vm.class_of(object)?;
        let mut at = class.id;
        while at != 0 {
            if let Some(m) = vm.methods(at)?.into_iter().find(|m| m.name == "toArray" && m.signature == "()[Ljava/lang/Object;") {
                return vm.invoke(thread, at, Some(object), m.id, &[]);
            }
            at = vm.superclass(at)?;
        }
        Ok(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_signatures() {
        assert_eq!(params("(IJLjava/lang/String;[I[[Ljava/lang/Object;)V"), ["I", "J", "Ljava/lang/String;", "[I", "[[Ljava/lang/Object;"]);
        assert!(params("()I").is_empty());
    }

    #[test]
    fn converts_arguments() {
        assert_eq!(convert(&Value::Int(3), "J"), Some(Value::Long(3)));
        assert_eq!(convert(&Value::Int(3), "D"), Some(Value::Double(3.0)));
        assert_eq!(convert(&Value::Char(97), "I"), Some(Value::Int(97)));
        assert_eq!(convert(&Value::Long(3), "I"), None);
        assert_eq!(convert(&Value::Double(1.5), "F"), None);
        assert_eq!(convert(&Value::Boolean(true), "I"), None);
    }
}
