//! Keep serde_json's wire format while bounding its uninterrupted string scan.
//! Composite values recursively use the same adapter; no intermediate JSON tree.

use serde::ser::{
    Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant, Serializer,
};
use std::{fmt, io, time::Instant};

const SLICE: usize = 4096;

pub(super) struct Checked<'a, T: ?Sized>(pub &'a T);

impl<T: Serialize + ?Sized> Serialize for Checked<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(Sliced(serializer))
    }
}

struct Text<'a>(&'a str);

impl fmt::Display for Text<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut rest = self.0;
        while !rest.is_empty() {
            let mut end = rest.len().min(SLICE);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            // serde_json escapes each fmt fragment separately. Its writer
            // checks the deadline before another fragment can be scanned.
            formatter.write_str(&rest[..end])?;
            rest = &rest[end..];
        }
        Ok(())
    }
}

struct Sliced<S>(S);
struct Compound<S>(S);

macro_rules! scalar {
    ($($name:ident($($arg:ident: $ty:ty),*));* $(;)?) => {$ (
        fn $name(self, $($arg: $ty),*) -> Result<Self::Ok, Self::Error> {
            self.0.$name($($arg),*)
        }
    )*};
}

impl<S: Serializer> Serializer for Sliced<S> {
    type Ok = S::Ok;
    type Error = S::Error;
    type SerializeSeq = Compound<S::SerializeSeq>;
    type SerializeTuple = Compound<S::SerializeTuple>;
    type SerializeTupleStruct = Compound<S::SerializeTupleStruct>;
    type SerializeTupleVariant = Compound<S::SerializeTupleVariant>;
    type SerializeMap = Compound<S::SerializeMap>;
    type SerializeStruct = Compound<S::SerializeStruct>;
    type SerializeStructVariant = Compound<S::SerializeStructVariant>;

    scalar! {
        serialize_bool(v: bool);
        serialize_i8(v: i8); serialize_i16(v: i16); serialize_i32(v: i32);
        serialize_i64(v: i64); serialize_i128(v: i128);
        serialize_u8(v: u8); serialize_u16(v: u16); serialize_u32(v: u32);
        serialize_u64(v: u64); serialize_u128(v: u128);
        serialize_f32(v: f32); serialize_f64(v: f64);
        serialize_char(v: char); serialize_bytes(v: &[u8]);
        serialize_none(); serialize_unit(); serialize_unit_struct(name: &'static str);
        serialize_unit_variant(name: &'static str, index: u32, variant: &'static str);
    }

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        self.0.collect_str(&Text(value))
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_some(&Checked(value))
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.0.serialize_newtype_struct(name, &Checked(value))
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.0
            .serialize_newtype_variant(name, index, variant, &Checked(value))
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        self.0.serialize_seq(len).map(Compound)
    }
    fn serialize_tuple(self, len: usize) -> Result<Self::SerializeTuple, Self::Error> {
        self.0.serialize_tuple(len).map(Compound)
    }
    fn serialize_tuple_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        self.0.serialize_tuple_struct(name, len).map(Compound)
    }
    fn serialize_tuple_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        self.0
            .serialize_tuple_variant(name, index, variant, len)
            .map(Compound)
    }
    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        self.0.serialize_map(len).map(Compound)
    }
    fn serialize_struct(
        self,
        name: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        self.0.serialize_struct(name, len).map(Compound)
    }
    fn serialize_struct_variant(
        self,
        name: &'static str,
        index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        self.0
            .serialize_struct_variant(name, index, variant, len)
            .map(Compound)
    }
    fn is_human_readable(&self) -> bool {
        self.0.is_human_readable()
    }
}

macro_rules! sequence {
    ($trait:ident, $method:ident) => {
        impl<S: $trait> $trait for Compound<S> {
            type Ok = S::Ok;
            type Error = S::Error;
            fn $method<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
                self.0.$method(&Checked(value))
            }
            fn end(self) -> Result<Self::Ok, Self::Error> {
                self.0.end()
            }
        }
    };
}
sequence!(SerializeSeq, serialize_element);
sequence!(SerializeTuple, serialize_element);
sequence!(SerializeTupleStruct, serialize_field);
sequence!(SerializeTupleVariant, serialize_field);

impl<S: SerializeMap> SerializeMap for Compound<S> {
    type Ok = S::Ok;
    type Error = S::Error;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Self::Error> {
        self.0.serialize_key(&Checked(key))
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Self::Error> {
        self.0.serialize_value(&Checked(value))
    }
    fn end(self) -> Result<Self::Ok, Self::Error> {
        self.0.end()
    }
}

macro_rules! structure {
    ($trait:ident) => {
        impl<S: $trait> $trait for Compound<S> {
            type Ok = S::Ok;
            type Error = S::Error;
            fn serialize_field<T: Serialize + ?Sized>(
                &mut self,
                key: &'static str,
                value: &T,
            ) -> Result<(), Self::Error> {
                self.0.serialize_field(key, &Checked(value))
            }
            fn skip_field(&mut self, key: &'static str) -> Result<(), Self::Error> {
                self.0.skip_field(key)
            }
            fn end(self) -> Result<Self::Ok, Self::Error> {
                self.0.end()
            }
        }
    };
}
structure!(SerializeStruct);
structure!(SerializeStructVariant);

/// serde_json's reader parser consults Read while scanning strings and
/// containers. Byte traversal checks every 4 KiB. The staged decoder also
/// avoids a whole internally-tagged Content tree conversion after the last
/// Read; checking this reader alone cannot bound that separate CPU tail.
pub(super) struct Decoding<'a> {
    pub bytes: &'a [u8],
    pub deadline: Instant,
    pub until_check: usize,
}

impl io::Read for Decoding<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.until_check == 0 || self.bytes.is_empty() {
            super::remaining(self.deadline)?;
            self.until_check = SLICE;
        }
        let count = self.bytes.len().min(output.len()).min(self.until_check);
        output[..count].copy_from_slice(&self.bytes[..count]);
        self.bytes = &self.bytes[count..];
        self.until_check -= count;
        Ok(count)
    }
}
