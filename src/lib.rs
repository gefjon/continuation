#![feature(never_type)]

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(PartialEq, Eq, Copy, Clone, Hash, Debug)]
/// A globally-unique thing with no properties other than total
/// equality.
///
/// To construct a continuation, we create a unique `Token`. The
/// continuation invokes an unwind with that token as its payload. The
/// resume point catches unwinds and compares their payload to its
/// token. If they are the same, it takes an action appropriate to the
/// continuation; if not, it re-throws the unwind.
struct Token(u64);

impl Token {
    fn next() -> Self {
        // Range of `u64` is large enough that wrapping is not a plausible
        // issue, so this counter will be monotonically increasing.
        static NEXT_TOKEN: AtomicU64 = AtomicU64::new(0);

        let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed);
        Token(token)
    }
}

/// Mimic the behavior of c's `setjmp`/`longjmp`.
///
/// Invokes `body` with two arguments: a payload and a repeat-continuation.
///
/// If `body` returns normally, `call_with_repeat_continuation`
/// returns its value.
///
/// The repeat continuation is a function of one argument which never
/// returns. When invoked, the repeat continuation causes `body` to be
/// re-invoked using the repeat continuation's argument as a new
/// payload.
///
/// For example, the following will print the numbers `1..=10`, then return `10`:
///
/// ```
/// # use continuation::call_with_repeat_continuation;
/// # let ten =
/// call_with_repeat_continuation(
///   0,
///   |i, repeat| {
///     println!("{}", i);
///     if i == 10 { i } else { repeat(i + 1) }
///   },
/// )
/// # ;
/// # assert_eq!(ten, 10);
/// ```
///
/// No convention is imposed as to the name of the repeat continuation.
pub fn call_with_repeat_continuation<Return, Payload, Body>(
    initial_payload: Payload,
    mut body: Body,
) -> Return
where
    Body: FnMut(Payload, &mut dyn FnMut(Payload) -> !) -> Return,
{
    let my_token = Token::next();
    // Ideally, this `Option` should be optimized away, since our
    // accesses to it strictly follow a loop of first initializing or
    // `replace`ing it, then `take`ing it. It would be easy to
    // manually remove the `Option` using `unsafe` with `MaybeUninit`,
    // but that would be kinda gross...
    let mut val = Some(initial_payload);
    'repeat: loop {
        match catch_unwind(
            // Because we're only catching our own unwinds, it
            // shouldn't be any more possible to break invariants than
            // normal, unless a buggy consumer invokes a repeat
            // continuation at a time when its invariants do not
            // hold. That would be their problem, not ours.
            AssertUnwindSafe(|| {
                let payload = val.take().unwrap();
                body(
                    payload,
                    &mut |new_payload| {
                        val.replace(new_payload);
                        // Use `resume_unwind` rather than `panic!` to
                        // avoid triggering the panic hook, since this
                        // isn't an actual panic.
                        resume_unwind(Box::new(my_token))
                    }
                )
            })) {
            Ok(ret) => return ret,
            Err(thrown_token) => {
                if let Some(&thrown_token) = thrown_token.downcast_ref::<Token>() {
                    if thrown_token == my_token {
                        // If this is our unwind, repeat the loop.
                        // note that the continuation will have
                        // already replaced `val` with a new payload.
                        continue 'repeat
                    }
                }
                // If we've caught someone else's unwind, bubble it up
                // the stack
                resume_unwind(thrown_token)
            }
        }
    }
}

/// Mimic C++'s exceptions locally.
///
/// Invokes `body` with one argument, an escape continuation.
///
/// If `body` returns normally, `call_with_escape_continuation`
/// returns that value wrapped in `Ok`.
///
/// The escape continuation is a function of one argument which never
/// returns. If the escape continuation is invoked,
/// `call_with_escape_continuation` will return its argument wrapped
/// in `Err`.
///
/// For example, the following expression returns `Ok(10)`:
///
/// ```
/// # use continuation::call_with_escape_continuation;
/// # let res =
/// call_with_escape_continuation(
///   |throw| if true { 5 + 5 } else { throw("unreachable") },
/// )
/// # ;
/// # assert_eq!(res, Ok(10));
/// ```
///
/// Whereas this expression returns `Err(10)`:
/// ```
/// # use continuation::call_with_escape_continuation;
/// # let res =
/// call_with_escape_continuation(
///   |throw| if false { "unreachable" } else { throw(20 - 10) },
/// )
/// # ;
/// # assert_eq!(res, Err(10));
/// ```
///
/// By convention, the escape continuation should be named `throw`, or
/// some variation thereof. Contexts with multiple nested
/// `call_with_escape_continuation` should each name their escape
/// continuations `throw_foo`, where `foo` describes the exceptional
/// situation; e.g. `throw_io_error`, `throw_thread_panicked`, etc.
pub fn call_with_escape_continuation<T, E, Body>(
    body: Body,
) -> Result<T, E>
where
    Body: FnOnce(&mut dyn FnMut(E) -> !) -> T,
{
    let mut body = Some(body);
    // There's likely a more efficient implementation not in terms of
    // `call_with_repeat_continuation`, but I don't feel like writing
    // it...
    call_with_repeat_continuation(
        None,
        move |error, throw| {
            if let Some(err) = error {
                // Second time through, after invoking the escape
                // continuation. Return the error.
                Err(err)
            } else if let Some(body) = body.take() {
                // First time through, with `error == initial_payload
                // == None`. Pass an escape continuation to `body`.
                Ok(body(&mut |err| throw(Some(err))))
            } else {
                // Third or more time through. Should be impossible.
                unreachable!("Loop in call/ec")
            }
        }
    )
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    /// In a bunch of concurrent threads, create a bunch of `Token`s
    /// and insert them into a `HashSet` to ensure they're unique
    fn tokens_unique() {
        use std::{collections::HashSet, thread};

        const N_THREADS: usize = 8;
        const TOKENS_PER_THREAD: usize = 1024;
        
        let mut threads = Vec::with_capacity(8);
        // first, each of `N_THREADS` threads creates
        // `TOKENS_PER_THREAD` tokens concurrently, without holding
        // any sort of shared lock or doing any synchronization beyond
        // that done by `Token::next`, and puts them in a local hash
        // set, erroring if any are duplicates.
        for _ in 0..N_THREADS {
            threads.push(thread::spawn(move || {
                let mut set = HashSet::with_capacity(TOKENS_PER_THREAD);
                for _ in 0..TOKENS_PER_THREAD {
                    let token = Token::next();
                    if !set.insert(token) {
                        return Err(token);
                    }
                }
                Ok(set)
            }));
        }

        // then, the main thread merges each of those sets into one
        // large set, erroring if there are any duplicates.
        let mut full_set = HashSet::with_capacity(TOKENS_PER_THREAD * N_THREADS);
        
        for thread in threads.drain(..) {
            let subset = thread.join()
                .expect("thread panicked")
                .expect("thread saw duplicate token");

            for token in subset.iter().copied() {
                if !full_set.insert(token) {
                    panic!("duplicate token while merging thread subsets");
                }
            }
        }
    }

    #[test]
    fn unused_callrepeat() {
        let zero = call_with_repeat_continuation(2, |two, _repeat| two - two);
        assert_eq!(zero, 0);
    }

    #[test]
    fn loop_callrepeat() {
        let kibi = call_with_repeat_continuation(
            0,
            |acc, repeat| if acc == 1024 { acc } else { repeat(acc + 1) }
        );
        assert_eq!(kibi, 1024)
    }

    #[test]
    fn nested_callrepeat() {
        let two = call_with_repeat_continuation(
            0,
            |outer_payload, outer_repeat| {
                if outer_payload == 2 { outer_payload } else {
                    call_with_repeat_continuation(
                        outer_payload,
                        |inner_payload, inner_repeat| {
                            if inner_payload % 2 == 0 {
                                inner_repeat(inner_payload + 1)
                            } else {
                                outer_repeat(inner_payload + 1)
                            }
                        }
                    )
                }
            }
        );
        assert_eq!(two, 2)
    }

    #[test]
    #[should_panic(expected = "test panic")]
    fn panicing_callrepeat() {
        call_with_repeat_continuation(
            0,
            |payload, repeat| {
                if payload == 10 {
                    panic!("test panic")
                } else {
                    repeat(payload + 1)
                }
            }
        )
    }

    #[test]
    fn unused_callec() {
        let zero: Result<i32, i32> = call_with_escape_continuation(
            |_throw| 1 - 1,
        );
        assert_eq!(zero, Ok(0))
    }

    #[test]
    fn throw_callec() {
        let zero: Result<i32, i32> = call_with_escape_continuation(
            |throw| throw(0),
        );
        assert_eq!(zero, Err(0))
    }

    #[test]
    fn callec_type_infer() {
        for i in 0..256 {
            let res = call_with_escape_continuation(
                |throw| if i % 2 == 0 { i } else { throw("odd!") }
            );
            if i % 2 == 0 {
                assert_eq!(res, Ok(i))
            } else {
                assert_eq!(res, Err("odd!"))
            }
        }
    }
}
