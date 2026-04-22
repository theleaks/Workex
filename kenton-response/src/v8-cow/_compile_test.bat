@echo off
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat" >nul
cl /nologo /std:c++17 /EHsc /Fe:cow_prototype.exe cow_builtins_prototype.cc
echo EXITCODE=%ERRORLEVEL%
