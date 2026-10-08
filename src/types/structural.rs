use core::fmt;
use std::sync::Arc;

use miniscript::iter::{Tree, TreeLike};
use simplicity::types::{CompleteBound, Final};

use super::{TypeConstructible, UIntType};
use crate::array::BTreeSlice;
use crate::num::NonZeroPow2Usize;

/// Internal structure of a SimplicityHL type.
///
/// 1:1 isomorphism to Simplicity.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct StructuralType(Arc<Final>);

impl AsRef<Final> for StructuralType {
    fn as_ref(&self) -> &Final {
        &self.0
    }
}

impl From<StructuralType> for Arc<Final> {
    fn from(value: StructuralType) -> Self {
        value.0
    }
}

impl From<Arc<Final>> for StructuralType {
    fn from(value: Arc<Final>) -> Self {
        Self(value)
    }
}

impl TreeLike for StructuralType {
    fn as_node(&self) -> Tree<Self> {
        match self.0.bound() {
            CompleteBound::Unit => Tree::Nullary,
            CompleteBound::Sum(l, r) | CompleteBound::Product(l, r) => {
                Tree::Binary(Self(l.clone()), Self(r.clone()))
            }
        }
    }
}

impl fmt::Debug for StructuralType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for StructuralType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<UIntType> for StructuralType {
    fn from(value: UIntType) -> Self {
        let inner = match value {
            UIntType::U1 => Final::two_two_n_fixed::<0>(),
            UIntType::U2 => Final::two_two_n_fixed::<1>(),
            UIntType::U4 => Final::two_two_n_fixed::<2>(),
            UIntType::U8 => Final::two_two_n_fixed::<3>(),
            UIntType::U16 => Final::two_two_n_fixed::<4>(),
            UIntType::U32 => Final::two_two_n_fixed::<5>(),
            UIntType::U64 => Final::two_two_n_fixed::<6>(),
            UIntType::U128 => Final::two_two_n_fixed::<7>(),
            UIntType::U256 => Final::two_two_n_fixed::<8>(),
        };
        Self(inner)
    }
}

impl TypeConstructible for StructuralType {
    fn either(left: Self, right: Self) -> Self {
        Self(Final::sum(left.0, right.0))
    }

    fn option(inner: Self) -> Self {
        Self::either(Self::unit(), inner)
    }

    fn boolean() -> Self {
        Self::either(Self::unit(), Self::unit())
    }

    fn tuple<I: IntoIterator<Item = Self>>(elements: I) -> Self {
        let elements: Vec<_> = elements.into_iter().collect();
        let tree = BTreeSlice::from_slice(&elements);
        tree.fold(Self::product).unwrap_or_else(Self::unit)
    }

    // Keep this implementation to prevent an infinite loop in <Self as TypeConstructible>::tuple
    fn unit() -> Self {
        Self(Final::unit())
    }

    // Keep this implementation to prevent an infinite loop in <Self as TypeConstructible>::tuple
    fn product(left: Self, right: Self) -> Self {
        Self(Final::product(left.0, right.0))
    }

    // Builds `size` copies in the shape of `BTreeSlice` from O(log size) shared subtrees.
    // Prevent OOM.
    fn array(element: Self, size: usize) -> Self {
        let bits = (usize::BITS - size.leading_zeros()) as usize;
        Self::pow2_arrays(element)
            .take(bits)
            .enumerate()
            .filter(|(i, _)| size >> i & 1 == 1)
            .map(|(_, array)| array)
            .reduce(Self::product)
            .unwrap_or_else(Self::unit)
    }

    // Shape of `Partition`: one optional block per power of two below `bound`, largest first.
    fn list(element: Self, bound: NonZeroPow2Usize) -> Self {
        Self::pow2_arrays(element)
            .take(bound.log2().get() as usize)
            .map(Self::option)
            .reduce(|smaller, block| Self::product(block, smaller))
            .expect("bound is at least 2")
    }
}

impl StructuralType {
    /// Arrays of `element` of size `2^0`, `2^1`, `2^2`, and so on.
    fn pow2_arrays(element: Self) -> impl Iterator<Item = Self> {
        std::iter::successors(Some(element), |array| {
            Some(Self::product(array.clone(), array.clone()))
        })
    }

    /// The balanced sum of the given leaf types.
    /// The structural type of an enum whose variants have these payload types.
    /// The tree shape is the one of [`BTreeSlice`], values ([`StructuralValue::enum_injection`])
    /// and the match lowering navigate the same shape.
    ///
    /// ## Panics
    ///
    /// `leaves` is empty: a sum of zero types would be uninhabited.
    ///
    /// [`StructuralValue::enum_injection`]: crate::value::StructuralValue
    pub(crate) fn balanced_sum(leaves: Vec<Self>) -> Self {
        BTreeSlice::from_slice(&leaves)
            .fold(Self::either)
            .expect("at least one leaf")
    }

    /// Convert into an unfinalized type that can be used in Simplicity's unification algorithm.
    pub fn to_unfinalized<'brand>(
        &self,
        inference_context: &simplicity::types::Context<'brand>,
    ) -> simplicity::types::Type<'brand> {
        simplicity::types::Type::complete(inference_context, self.0.clone())
    }
}
