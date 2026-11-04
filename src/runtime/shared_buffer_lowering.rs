//! Native lowering of `SharedBuffer:readf64` and `readu32`.
//!
//! In `--!native` code with the receiver annotated (`local view: jess_runtime_SharedBuffer`), a
//! read is a tag check on the receiver, an exact-integer and range check on the offset, and one
//! load: no call, and no copy of the payload. Anything else (a wrong receiver, a fractional or
//! out-of-range offset, an unannotated call) leaves for the interpreter and the ordinary method,
//! so an error is the same error either way.
//!
//! # How native code reaches the bytes
//!
//! Luau's IR has no load from an arbitrary address. Its buffer loads add `offsetof(Udata, data)`
//! to their base under the userdata tag, from whatever register they are given, so an address
//! less that offset is a base. l3i's own intern lowering reads a pool's arrays the same way, and
//! pins the offset against Luau's headers when it builds.
//!
//! Every part of that contract breaks the build rather than a run. This module pins the payload's
//! layout with `offset_of!`. l3i doesn't export the header's size, so [`VERIFIED_LUAU`] names the
//! Luau release it was read from, and a jess on any other Luau refuses to compile until someone
//! reads `offsetof(Udata, data)` in that release's `VM/src/lobject.h` again.
//!
//! # Why the address can't dangle
//!
//! The payload's address and length never change once a view exists, and they point into
//! storage the view's `Arc` keeps alive. The receiver stays in its VM register throughout: no
//! instruction between loading the address and loading through it can run Luau, allocate, or
//! collect, so the payload can't be finalized in between. Generated code keeps no address after
//! the read, and nothing hands one to a script. Storage is never written once sealed, so readers
//! on other threads need no lock.

use std::cell::Cell;

use l3i::native_code::NativeCodeHooks;
use l3i::native_code::hooks::{NamecallSite, NativeContext};
use l3i::native_code::ir::{IrBuilder, IrCmd, IrCondition, IrOp, bytecode_type};
use l3i::stack::Type;

use crate::runtime::shared_buffer::{PAYLOAD_ADDRESS, PAYLOAD_LENGTH, SharedBuffer};

const USERDATA_TAG: u8 = Type::Userdata as u8;
const NUMBER_TAG: u8 = Type::Number as u8;

/// `offsetof(Udata, data)`: what a buffer load adds under the userdata tag.
const UDATA_DATA: i64 = 16;

/// The Luau release [`UDATA_DATA`] was read from.
const VERIFIED_LUAU: &str = "0.744";

const fn same_text(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }

    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }

        index += 1;
    }

    true
}

const _: () = assert!(
    same_text(l3i::LUAU_VERSION, VERIFIED_LUAU),
    "Luau changed: check offsetof(Udata, data) in VM/src/lobject.h against UDATA_DATA, then update VERIFIED_LUAU"
);

// Per thread: a test compares these before and after, and tests run in parallel.
thread_local! {
    static LOWERED: Cell<usize> = const { Cell::new(0) };
    static METHOD_READS: Cell<usize> = const { Cell::new(0) };
}

/// How many read sites the hook has lowered on this thread (a diagnostic for tests).
#[doc(hidden)]
#[must_use]
pub fn lowered_sites() -> usize {
    LOWERED.get()
}

/// How many reads ran through the method on this thread: what lowering avoids (a diagnostic for
/// tests, always zero in a release build).
#[doc(hidden)]
#[must_use]
pub fn method_reads() -> usize {
    METHOD_READS.get()
}

#[inline]
pub(crate) fn count_method_read() {
    if cfg!(debug_assertions) {
        METHOD_READS.set(METHOD_READS.get() + 1);
    }
}

#[derive(Clone, Copy)]
enum Read {
    F64,
    U32,
}

impl Read {
    fn of(member: &str) -> Option<Read> {
        match member {
            "readf64" => Some(Read::F64),
            "readu32" => Some(Read::U32),
            _ => None,
        }
    }

    const fn width(self) -> i64 {
        match self {
            Read::F64 => 8,
            Read::U32 => 4,
        }
    }
}

/// The hook set; the runtime extension registers it.
pub(crate) struct SharedBufferLowering;

// Only where these loads are little-endian and the header is the size pinned above. Elsewhere
// every read stays on the method, which is correct everywhere.
const SUPPORTED: bool = cfg!(target_endian = "little") && cfg!(target_pointer_width = "64");

fn payload_word(build: &mut IrBuilder<'_>, userdata: IrOp, offset: usize) -> IrOp {
    let offset = build.const_int(i32::try_from(offset).expect("a payload offset fits"));
    let tag = build.const_tag(USERDATA_TAG);
    build.inst(IrCmd::BUFFER_READI64, &[userdata, offset, tag])
}

impl NativeCodeHooks for SharedBufferLowering {
    fn userdata_namecall_type(&self, context: &NativeContext<'_>, userdata_type: u8, member: &str) -> u8 {
        if !SUPPORTED || context.userdata_type_of::<SharedBuffer>() != Some(userdata_type) {
            return bytecode_type::ANY;
        }

        Read::of(member).map_or(bytecode_type::ANY, |_| bytecode_type::NUMBER)
    }

    fn userdata_namecall(
        &self,
        context: &NativeContext<'_>,
        build: &mut IrBuilder<'_>,
        userdata_type: u8,
        member: &str,
        site: NamecallSite,
    ) -> bool {
        if !SUPPORTED || context.userdata_type_of::<SharedBuffer>() != Some(userdata_type) {
            return false;
        }

        let Some(read) = Read::of(member) else { return false };
        if site.params != 2 || site.results != 1 {
            return false;
        }

        let Some(tag) = context.tag_of::<SharedBuffer>() else {
            return false;
        };
        let exit = build.vm_exit(site.pcpos);
        let receiver = build.vm_reg(site.source_reg);
        let userdata = build.inst(IrCmd::LOAD_POINTER, &[receiver]);
        let tag = build.const_int(i32::from(tag));
        build.inst(IrCmd::CHECK_USERDATA_TAG, &[userdata, tag, exit]);

        // A whole number survives the round trip; NaN, infinities and fractions don't, and the
        // conversion's arch-specific result for anything too large fails the range check below.
        let offset_reg = build.vm_reg(site.arg_res_reg + 2);
        build.load_and_check_tag(offset_reg, NUMBER_TAG, exit);
        let number = build.inst(IrCmd::LOAD_DOUBLE, &[offset_reg]);
        let offset = build.inst(IrCmd::NUM_TO_INT64, &[number]);
        let back = build.inst(IrCmd::INT64_TO_NUM, &[offset]);
        let equal = build.cond(IrCondition::Equal);
        build.inst(IrCmd::CHECK_CMP_NUM, &[back, number, equal, exit]);

        // Signed, so a view shorter than the read has a negative last offset and nothing fits.
        let length = payload_word(build, userdata, PAYLOAD_LENGTH);
        let width = build.const_int64(read.width());
        let last = build.inst(IrCmd::SUB_INT64, &[length, width]);
        let zero = build.const_int64(0);
        let at_least = build.cond(IrCondition::GreaterEqual);
        build.inst(IrCmd::CHECK_CMP_INT64, &[offset, zero, at_least, exit]);
        let at_most = build.cond(IrCondition::LessEqual);
        build.inst(IrCmd::CHECK_CMP_INT64, &[offset, last, at_most, exit]);

        let address = payload_word(build, userdata, PAYLOAD_ADDRESS);
        let at = build.inst(IrCmd::ADD_INT64, &[address, offset]);
        let header = build.const_int64(UDATA_DATA);
        let base = build.inst(IrCmd::SUB_INT64, &[at, header]);
        let index = build.const_int(0);
        let tag = build.const_tag(USERDATA_TAG);
        let value = match read {
            Read::F64 => build.inst(IrCmd::BUFFER_READF64, &[base, index, tag]),
            Read::U32 => {
                let raw = build.inst(IrCmd::BUFFER_READI32, &[base, index, tag]);
                build.inst(IrCmd::UINT_TO_NUM, &[raw])
            }
        };

        let result = build.vm_reg(site.arg_res_reg);
        build.inst(IrCmd::STORE_DOUBLE, &[result, value]);
        let number_tag = build.const_tag(NUMBER_TAG);
        build.inst(IrCmd::STORE_TAG, &[result, number_tag]);
        LOWERED.set(LOWERED.get() + 1);
        true
    }
}
