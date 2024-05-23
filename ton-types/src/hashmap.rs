use std::collections::HashMap;
use crate::bag_of_cells::{BagOfCells, CellInBag};
use crate::cell::CellId;

#[derive(Clone, Debug)]
pub struct Slice<'a> {
    bag: &'a BagOfCells,
    children: &'a [CellId],
    data: &'a [u8],
    cnt: usize
}

#[derive(Debug)]
pub enum Error {}

impl<'a> Slice<'a> {
    pub fn new(CellInBag { cell, bag }: CellInBag<'a>) -> Self {
        let children = cell.refs();
        let data = cell.as_ref();

        Slice { bag, children, data, cnt: 0 }
    }

    pub fn read_bit(&mut self) -> Result<bool, Error> {
        let bit: bool;
        ((self.data, self.cnt), bit) = nom::bits::complete::bool::<&[u8], ()>((self.data, self.cnt)).unwrap();

        Ok(bit)
    }

    pub fn read_bits(&mut self, n: usize) -> Result<usize, Error> {
        let m: usize;
        ((self.data, self.cnt), m) = nom::bits::complete::take::<&[u8], _, _, ()>(n)((self.data, self.cnt)).unwrap();

        Ok(m.to_le())
    }

    pub fn take_child_cell(&mut self) -> Result<CellInBag<'a>, Error> {
        let (cell_id, tail) = self.children.split_first().unwrap();
        self.children = tail;

        let cell = self.bag.get(*cell_id as usize).unwrap();

        Ok(cell)
    }
}

pub(crate) trait FromBitReader: Sized {
    fn from_bit_reader(input: &mut Slice) -> Result<Self, Error>;
}

pub struct Unary {
    n: u32
}

impl FromBitReader for Unary {
    fn from_bit_reader(input: &mut Slice) -> Result<Self, Error> {
        let mut input = input;
        let mut n = 0;

        while input.read_bit()? {
            n += 1;
        }

        Ok(Self { n })
    }
}

#[derive(Debug)]
pub struct HmLabel {
    n: u32,
    m: u32,
    label: u32
}

impl HmLabel {
    pub fn read(m: u32, input: &mut Slice) -> Result<Self, Error> {
        let bit = input.read_bit()?;
        if bit {
            let bit = input.read_bit()?;
            if bit {
                let bit = input.read_bit()?;
                let len = input.read_bits(len_bits(m) as usize)? as u32;

                if bit {
                    Ok(Self { label: (1u32 << len) - 1, m, n: len })
                } else {
                    Ok(Self { label: 0, m, n: len })
                }
            } else {
                let len = input.read_bits(len_bits(m) as usize)? as u32;
                let label = input.read_bits(len as usize)? as u32;

                Ok(Self { label, m, n: len })
            }
        } else {
            let Unary { n: len} = Unary::from_bit_reader(input)?;
            let label = input.read_bits(len as usize)? as u32;

            Ok(Self { label, m, n: len })
        }
    }
}

const fn len_bits(value: u32) -> u32 {
    32 - value.leading_zeros()
}

struct Hashmap<X> {
    label: HmLabel,
    hashmap_node: HashmapNode<X>
}

pub enum HashmapNode<X> {
    Leaf { value: X },
    Fork { left: CellId, right: CellId }
}

#[derive(Default, Debug)]
struct HashmapE<const K: u32, X> {
    inner: HashMap<u32, X>
}

impl<const K: u32, X> HashmapE<K, X> where X: FromBitReader {
    fn parse(input: &mut Slice) -> Result<Self, Error> {
        let mut inner = HashMap::new();

        let bit = input.read_bit()?;
        if bit {
            let root = input.take_child_cell()?;
            println!("root non-empty: {:?}", root);

            let mut input = Slice::new(root);
            let label = HmLabel::read(K, &mut input).unwrap();
            println!("label: {:?}", label);

            let m = K - label.n;
            println!("m: {:?}", m);
            if m > 0 {
                let left = input.take_child_cell()?;

                println!("left: {:?}", left);
                for c in left.children() {
                    println!("c: {:?}", c);
                }

                let right = input.take_child_cell()?;
                println!("right: {:?}", right);
            } else {
                let v = X::from_bit_reader(&mut input)?;
                inner.insert(label.label, v);
            }

            Ok(Self { inner })
        } else {
            Ok(Self { inner: Default::default() })
        }
    }
}

/**
hme_empty$0 {n:#} {X:Type} = HashmapE n X;
hme_root$1 {n:#} {X:Type} root:^(Hashmap n X) = HashmapE n X

hm_edge#_ {n:#} {X:Type} {l:#} {m:#} label:(HmLabel ~l n)
          {n = (~m) + l} node:(HashmapNode m X) = Hashmap n X;

hmn_leaf#_ {X:Type} value:X = HashmapNode 0 X;
hmn_fork#_ {n:#} {X:Type} left:^(Hashmap n X)
           right:^(Hashmap n X) = HashmapNode (n + 1) X;

hml_short$0 {m:#} {n:#} len:(Unary ~n) {n <= m} s:(n * Bit) = HmLabel ~n m;
hml_long$10 {m:#} n:(#<= m) s:(n * Bit) = HmLabel ~n m;
hml_same$11 {m:#} v:Bit n:(#<= m) = HmLabel ~n m;


_ (HashmapE 32 ^(BinTree ShardDescr)) = ShardHashes;

bt_leaf$0 {X:Type} leaf:X = BinTree X;
bt_fork$1 {X:Type} left:^(BinTree X) right:^(BinTree X)
          = BinTree X;

**/


#[derive(Debug)]
pub struct BinTree<X> {
    inner: Vec<X>
}

impl<X> FromBitReader for BinTree<X> where X : FromBitReader {
    fn from_bit_reader(input: &mut Slice) -> Result<Self, Error> {
        let mut output = Vec::new();
        let mut stack = Vec::new();
        stack.push(input.to_owned());

        while let Some(mut current_cell) = stack.pop() {
            let bit = current_cell.read_bit()?;
            if bit {
                let left = current_cell.take_child_cell()?;
                let left = Slice::new(left);
                stack.push(left);

                let right = current_cell.take_child_cell()?;
                let right = Slice::new(right);
                stack.push(right);
            } else {
                let value = X::from_bit_reader(&mut current_cell)?;

                output.push(value);
            }
        }

        Ok(Self { inner: output })
    }
}

#[derive(Debug)]
struct ChildCell<X> {
    pub(crate) inner: X
}

impl<X> FromBitReader for ChildCell<X> where X: FromBitReader {
    fn from_bit_reader(input: &mut Slice) -> Result<Self, Error> {
        let cell = input.take_child_cell()?;
        let mut slice = Slice::new(cell);

        let inner = X::from_bit_reader(&mut slice)?;

        Ok(Self { inner })
    }
}

#[cfg(test)]
mod tests {
    use nom::complete::bool;
    use nom::IResult;
    use crate::bag_of_cells::BagOfCells;
    use crate::cell::Cell;
    use crate::deserializer::{BitInput, from_bytes};
    use crate::hashmap::{BinTree, ChildCell, FromBitReader, HashmapE, HmLabel, Slice, Unary};
    use crate::shard_descr::ShardDescr;

    #[test]
    fn parser_ordering_test() {
        let n = 0b10000000;

        assert_eq!(128, n);
    }

    fn given_boc(content: Vec<u8>) -> BagOfCells {
        let cell = Cell::new(content, vec![]);

        BagOfCells::new(vec![cell])
    }

    #[test]
    fn unary_zero_test() {
        let input = vec![0_u8];
        let boc = given_boc(input);
        let mut slice = Slice::new(boc.root().unwrap());

        let unary = Unary::from_bit_reader(&mut slice).unwrap();

        assert_eq!(unary.n, 0);
    }

    #[test]
    fn unary_succ_test() {
        let input = vec![0b11100000_u8];
        let boc = given_boc(input);
        let mut slice = Slice::new(boc.root().unwrap());

        let unary = Unary::from_bit_reader(&mut slice).unwrap();

        assert_eq!(unary.n, 3);
    }

    #[test]
    fn hmlabel_short_test() {
        let input = vec![0b0_111_0101_u8];
        let boc = given_boc(input);
        let mut slice = Slice::new(boc.root().unwrap());

        let label = HmLabel::read(32, &mut slice).unwrap();

        assert_eq!(label.label, 0b00000101);
        assert_eq!(label.m, 32);
        assert_eq!(label.n, 3);
    }

    #[test]
    fn hmlabel_long_test() {
        let input = vec![0b10_011_101_u8];
        let boc = given_boc(input);
        let mut slice = Slice::new(boc.root().unwrap());

        let label = HmLabel::read(4, &mut slice).unwrap();

        assert_eq!(label.label, 0b00000101);
        assert_eq!(label.m, 4);
        assert_eq!(label.n, 3);
    }

    #[test]
    fn hmlabel_same_test() {
        let input = vec![0b11_1_01000_u8];
        let boc = given_boc(input);
        let mut slice = Slice::new(boc.root().unwrap());

        let label = HmLabel::read(16, &mut slice).unwrap();

        assert_eq!(label.label, 0b11111111);
        assert_eq!(label.m, 16);
        assert_eq!(label.n, 8);
    }

    #[test]
    fn hmlabel_same_real_test() {
        let input = hex::decode("d000").unwrap();
        let boc = given_boc(input);
        let mut slice = Slice::new(boc.root().unwrap());

        let label = HmLabel::read(32, &mut slice).unwrap();

        assert_eq!(label.label, 0);
        assert_eq!(label.m, 32);
        assert_eq!(label.n, 32);
    }

    #[test]
    fn shard_hashes_test() {
        let bytes = hex::decode("b5ee9c7201020701000110000101c0010103d040020201c0030401eb5014c376901214cdb0000152890a35b600000152890a35b85e31d8be7f5f1b44600e445b3cf778b40eaad885db5153838bea3e8f0f4a9b25e36422b74bfadf372f7d3e16b48c05f4866b05d2c7e5787bd954a5d79ad9fdb6990000450f5a00000000000000001214cd933228b81ccc8a2e52000000c90501db5014c367381214cda8000152890aafc800000152890aafcefff0db0738592205986066e14fa1221d28f0156604fd4346cea0b705712ddd2872d9dc6b6fd4eb6624bf6cb9b77d673d2df07a993f5ed281b375f3c659c25e4df80000450f5e00000000000000001214cd933228b8020600134591048ab20ee6b28020001343332bfa820ee6b28020").unwrap();
        let boc = from_bytes::<BagOfCells>(&bytes).unwrap();
        let root = boc.root().unwrap();
        println!("root: {:?}", root);

        // _ (HashmapE 32 ^(BinTree ShardDescr)) = ShardHashes;
        let hashmap = HashmapE::<32, ChildCell<BinTree<ShardDescr>>>::parse(&mut Slice::new(root)).unwrap();
        println!("output: {:?}", hashmap);

        assert_eq!(hashmap.inner.len(), 1);
        assert_eq!(hashmap.inner.get(&0_u32).unwrap().inner.inner.len(), 2);
    }
}
