// Schema types for runtime channel binding.
//
// These types describe the structure of method arguments so the runtime
// binder can find and bind Tx/Rx channels.

/** Primitive types that don't need binding. */
export type PrimitiveKind =
  | 'bool'
  | 'u8' | 'u16' | 'u32' | 'u64'
  | 'i8' | 'i16' | 'i32' | 'i64'
  | 'f32' | 'f64'
  | 'string' | 'bytes';

/** Schema for Tx<T> - needs binding for outgoing data. */
export interface TxSchema {
  kind: 'tx';
  element: Schema;
}

/** Schema for Rx<T> - needs binding for incoming data. */
export interface RxSchema {
  kind: 'rx';
  element: Schema;
}

/** Schema for Vec<T>. */
export interface VecSchema {
  kind: 'vec';
  element: Schema;
}

/** Schema for Option<T>. */
export interface OptionSchema {
  kind: 'option';
  inner: Schema;
}

/** Schema for HashMap<K, V>. */
export interface MapSchema {
  kind: 'map';
  key: Schema;
  value: Schema;
}

/** Schema for a struct with named fields. */
export interface StructSchema {
  kind: 'struct';
  fields: Record<string, Schema>;
}

/** Schema for an enum with variants. */
export interface EnumSchema {
  kind: 'enum';
  variants: Record<string, Schema[]>;  // variant name -> tuple of field schemas
}

/** Union of all schema types. */
export type Schema =
  | { kind: PrimitiveKind }
  | TxSchema
  | RxSchema
  | VecSchema
  | OptionSchema
  | MapSchema
  | StructSchema
  | EnumSchema;

/** Schema for a method's arguments. */
export interface MethodSchema {
  args: Schema[];
}
