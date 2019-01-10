pub struct SymbolicExpression {
    value: String,
    children: Option<Box<[SymbolicExpression]>>
}

pub struct Contract {
    content: Box<[SymbolicExpression]>
}

pub struct IntValueType (u64);

pub struct BoolValueType (bool);

pub struct BufferType (Box<[char]>);

#[derive(Debug)]
pub enum ValueType {
    IntType(u64),
    BoolType(bool),
    BufferType(Box<[char]>),
    IntListType(Vec<u64>),
    BoolListType(Vec<bool>),
    BufferListType(Vec<Box<[char]>>)
}

fn parseInteger(value: &ValueType) -> u64 {
    match *value {
        ValueType::IntType(int) => int,
        _ => panic!("Not an integer")
    }
}

fn nativeAdd(args: &[ValueType]) -> ValueType {
    let parsedArgs = args.iter().map(|x| parseInteger(x));
    let result = parsedArgs.fold(0, |acc, x| acc + x);
    ValueType::IntType(result)
}

fn lookupVariable(name: &str) -> ValueType {
    // first off, are we talking about a constant?
    if name.starts_with(char::is_numeric) {
        match u64::from_str_radix(name, 10) {
            Ok(parsed) => ValueType::IntType(parsed),
            Err(_e) => panic!("Failed to parse!")
        }
    } else {
        panic!("Not implemented");
    }
}

fn lookupFunction(name: &str)-> fn(&[ValueType]) -> ValueType {
    match name {
        "+" => nativeAdd,
        _ => panic!("Crash and burn")
    }
}

fn apply<F>(function: &F, args: &[SymbolicExpression]) -> ValueType
    where F: Fn(&[ValueType]) -> ValueType {
    let evaluatedArgs: Vec<ValueType> = args.iter().map(|x| eval(x)).collect();
    function(&evaluatedArgs)
}

fn eval(exp: &SymbolicExpression) -> ValueType {
    match exp.children {
        None => lookupVariable(&exp.value),
        Some(ref children) => {
            let f = lookupFunction(&exp.value);
            apply(&f, &children)
        }
    }
}

fn main() {
    let content = [ SymbolicExpression { value: "+".to_string(),
                                         children:
                                         Some(Box::new([ SymbolicExpression { value: "1".to_string(),
                                                                              children: None },
                                                         SymbolicExpression { value: "1".to_string(),
                                                                              children: None } ])) } ];
//    let contract = Contract { content: Box::new(content) } ;
    println!("{:?}", eval(&content[0]));
}
