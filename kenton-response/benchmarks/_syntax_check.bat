@echo off
setlocal
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
set STD=/std:c++20 /Zc:__cplusplus
set INC=/I "..\vendor\v8-headers-test"
set DEFS=/DV8_COMPRESS_POINTERS /DV8_ENABLE_SANDBOX /D_HAS_CXX20=1

echo === syntax-check p1_serialization_cost.cc ===
cl /nologo %STD% /EHsc /Zs /W3 %INC% %DEFS% p1_serialization_cost.cc > p1_syntax.log 2>&1
echo EXIT=%ERRORLEVEL%
type p1_syntax.log

echo.
echo === syntax-check p2_resume_cost.cc ===
cl /nologo %STD% /EHsc /Zs /W3 %INC% %DEFS% p2_resume_cost.cc > p2_syntax.log 2>&1
echo EXIT=%ERRORLEVEL%
type p2_syntax.log

echo.
echo === syntax-check common.cc ===
cl /nologo %STD% /EHsc /Zs /W3 %INC% %DEFS% common.cc > common_syntax.log 2>&1
echo EXIT=%ERRORLEVEL%
type common_syntax.log
