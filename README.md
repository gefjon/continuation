# Continuation
## Pure safe Rust implementations of continuation-based control flow

This crate exports two functions, `call_with_repeat_continuation` and
`call_with_escape_continuation`. The former behaves similarly to C's `setjmp`/`longjmp`,
and the latter gives access to C++-style exceptions within a local scope.

## Why?

Honestly, I wanted to see if this was possible in safe Rust. It turns out, in the presence
of `std` and `panic = "unwind"`, it is, by using `catch_unwind` and `resume_unwind`! As
per [The Rustonomicon](https://doc.rust-lang.org/nomicon/unwinding.html), unwinding via
the `panic` mechanism is likely to be wildly inefficient compared to exceptions in
languages like C++ or Java. As a result, this crate is not suitable for high-performance
use, and is frankly not useful for anything in particular. It's kinda cool, though.

## License

This code is released under the MIT license, as it appears in [LICENSE.md](./LICENSE.md)
or at [the Open Source Initiative](https://opensource.org/licenses/MIT). In my mind, that
means I don't care what you do with the code and that I don't have any obligation to
support you in so doing. That said, if you do something cool, I'd love to hear about it!
