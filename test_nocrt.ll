target datalayout = "e-m:w-p270:32:32-p271:32:32-p272:64:64-i64:64-f80:128-n8:16:32:64-S128"
target triple = "x86_64-pc-windows-msvc"

declare dllimport ptr @GetStdHandle(i32)
declare dllimport i32 @WriteConsoleA(ptr, ptr, i32, ptr, ptr)
declare dllimport void @ExitProcess(i32)

@str = private unnamed_addr constant [7 x i8] c"hello\0A\00", align 1

define void @mainCRTStartup() {
  %hStdOut = call ptr @GetStdHandle(i32 -11)
  %written = alloca i32, align 4
  %ret = call i32 @WriteConsoleA(ptr %hStdOut, ptr @str, i32 6, ptr %written, ptr null)
  call void @ExitProcess(i32 0)
  unreachable
}
