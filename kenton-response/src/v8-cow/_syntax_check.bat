@echo off
setlocal
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
set STD=/std:c++20 /Zc:__cplusplus
set INC=/I "..\..\vendor\v8-headers-test"
set DEFS=/DV8_COMPRESS_POINTERS /DV8_ENABLE_SANDBOX

echo === syntax-check memory_benchmark.cc ===
cl /nologo %STD% /EHsc /Zs /W3 %INC% %DEFS% memory_benchmark.cc > memory_syntax.log 2>&1
echo EXIT=%ERRORLEVEL%
type memory_syntax.log

echo.
echo === syntax-check heap_classifier.cc ===
cl /nologo %STD% /EHsc /Zs /W3 %INC% %DEFS% heap_classifier.cc > heap_syntax.log 2>&1
echo EXIT=%ERRORLEVEL%
type heap_syntax.log

echo.
echo === syntax-check cow_builtins_prototype.cc ===
cl /nologo %STD% /EHsc /Zs /W3 cow_builtins_prototype.cc > proto_syntax.log 2>&1
echo EXIT=%ERRORLEVEL%
type proto_syntax.log
