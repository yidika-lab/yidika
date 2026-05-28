#include <llvm-c/Core.h>
#include <llvm-c/BitReader.h>

int main() {
    LLVMContextRef ctx = LLVMContextCreate();
    LLVMMemoryBufferRef buf;
    char* err = NULL;
    const char* ir = "define i32 @main() { ret i32 0 }";
    LLVMCreateMemoryBufferWithMemoryRange(ir, strlen(ir), "test", 0);
    printf("OK\\n");
    LLVMContextDispose(ctx);
    return 0;
}
