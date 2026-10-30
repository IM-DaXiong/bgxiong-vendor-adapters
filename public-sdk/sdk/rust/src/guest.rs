use crate::{Invocation,Response,Host};
use serde::Serialize;
use serde_json::json;
use core::marker::PhantomData;

pub trait Adapter {
    fn invoke(request:Invocation,host:&mut dyn Host)->Response;
}

/// Canonical portable package paths; this is shared by authoring and workers.
pub fn valid_package_path(path:&str)->bool{
    !path.is_empty()&&!path.starts_with('/')&&!path.contains(['\\',':'])&&!path.chars().any(char::is_control)
        &&path.split('/').all(|part|!part.is_empty()&&part!="."&&part!="..")
}
pub fn parse_hash_inventory(source:&str)->Result<alloc::collections::BTreeMap<alloc::string::String,alloc::string::String>,&'static str>{
    let mut files=alloc::collections::BTreeMap::new();
    for line in source.lines(){let(hash,file)=line.split_once(' ').ok_or("Invalid package inventory line")?;
        if hash.len()!=64||!hash.bytes().all(|c|c.is_ascii_digit()||(b'a'..=b'f').contains(&c))||!valid_package_path(file)
            ||files.insert(file.into(),hash.into()).is_some(){return Err("Invalid, duplicate or unsafe package inventory entry");}}
    Ok(files)
}

pub fn completed_response<T:Serialize>(request:&Invocation,data:&T)->Result<Response,crate::host::HostError>{
    let payload=serde_json::to_string(data).map_err(|_|crate::host::invalid_wire("Result cannot be encoded"))?;
    serde_json::from_value(json!({"protocolVersion":2,"operation":request.r#operation,"dataJson":payload}))
        .map_err(|_|crate::host::invalid_wire("Result envelope is invalid"))
}

#[cfg(feature="component-guest")]
pub struct ComponentAdapter<A>(PhantomData<A>);
#[cfg(feature="component-guest")]
impl<A:Adapter> crate::__bindings::Guest for ComponentAdapter<A>{
    fn invoke(request:crate::__bindings::Invocation)->crate::__bindings::Response{
        use crate::generated::wire_abi as abi;
        let operation=abi::from_invocation_operation(request.operation).expect("closed operation enum");
        let response=match abi::from_invocation(request){
            Ok(request)=>A::invoke(request,&mut crate::host::ComponentHost),
            Err(error)=>serde_json::from_value(json!({"protocolVersion":2,"operation":operation,"error":{
                "code":"INVALID_INPUT","message":error.r#message,"retryable":false,"vendorCode":null,"vendorRequestId":null}}))
                .expect("published response shape"),
        };
        abi::to_response(response).expect("typed response has only supported ABI values")
    }
}

/// Export the single invoke entry, no_std allocation and C ABI memory routines.
/// The worker enforces memory/fuel limits. The adapter uses no_std on wasm32.
#[cfg(feature="component-guest")]
#[macro_export]
macro_rules! export_adapter {
    ($adapter:ty) => {
        type __BgxComponent = $crate::guest::ComponentAdapter<$adapter>;
        $crate::__bindings::export!(__BgxComponent with_types_in $crate::__bindings);
        #[cfg(target_arch="wasm32")]
        #[global_allocator]
        static __BGX_ALLOCATOR:$crate::__dlmalloc::GlobalDlmalloc=$crate::__dlmalloc::GlobalDlmalloc;
        #[cfg(target_arch="wasm32")]
        #[panic_handler]
        fn __bgx_panic(_: &core::panic::PanicInfo<'_>)->!{core::arch::wasm32::unreachable()}
        #[cfg(target_arch="wasm32")]
        #[no_mangle]
        pub unsafe extern "C" fn memcpy(d:*mut u8,s:*const u8,n:usize)->*mut u8{$crate::guest::memory::copy(d,s,n)}
        #[cfg(target_arch="wasm32")]
        #[no_mangle]
        pub unsafe extern "C" fn memmove(d:*mut u8,s:*const u8,n:usize)->*mut u8{$crate::guest::memory::move_bytes(d,s,n)}
        #[cfg(target_arch="wasm32")]
        #[no_mangle]
        pub unsafe extern "C" fn memset(d:*mut u8,c:i32,n:usize)->*mut u8{$crate::guest::memory::set(d,c as u8,n)}
        #[cfg(target_arch="wasm32")]
        #[no_mangle]
        pub unsafe extern "C" fn memcmp(a:*const u8,b:*const u8,n:usize)->i32{$crate::guest::memory::compare(a,b,n)}
        #[cfg(target_arch="wasm32")]
        #[no_mangle]
        pub unsafe extern "C" fn cabi_realloc(p:*mut u8,old:usize,align:usize,new:usize)->*mut u8{$crate::guest::memory::reallocate(p,old,align,new)}
    };
}

#[doc(hidden)]
pub mod memory {
    use alloc::alloc::{alloc,dealloc,realloc,Layout};
    pub unsafe fn copy(destination:*mut u8,source:*const u8,length:usize)->*mut u8{
        for index in 0..length{destination.add(index).write_volatile(source.add(index).read_volatile());}destination
    }
    pub unsafe fn move_bytes(destination:*mut u8,source:*const u8,length:usize)->*mut u8{
        if (destination as usize)<(source as usize){copy(destination,source,length)}else{
            for index in (0..length).rev(){destination.add(index).write_volatile(source.add(index).read_volatile());}destination}
    }
    pub unsafe fn set(destination:*mut u8,value:u8,length:usize)->*mut u8{
        for index in 0..length{destination.add(index).write_volatile(value);}destination
    }
    pub unsafe fn compare(a:*const u8,b:*const u8,length:usize)->i32{
        for index in 0..length{let left=a.add(index).read_volatile();let right=b.add(index).read_volatile();if left!=right{return i32::from(left)-i32::from(right);}}0
    }
    pub unsafe fn reallocate(pointer:*mut u8,old:usize,alignment:usize,new:usize)->*mut u8{
        let layout=|size|Layout::from_size_align(size,alignment).expect("invalid component allocation layout");
        if new==0{if old!=0{dealloc(pointer,layout(old));}return alignment as *mut u8;}
        let output=if old==0{alloc(layout(new))}else{realloc(pointer,layout(old),new)};
        if output.is_null(){alloc::alloc::handle_alloc_error(layout(new));}output
    }
}
