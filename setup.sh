#!/usr/bin/env bash

if [ "$(uname)" == "Darwin" ]; then
    NPROC=$(sysctl -n hw.physicalcpu)  
elif [ "$(expr substr $(uname -s) 1 5)" == "Linux" ]; then
    NPROC=$(nproc)  
fi

python3 -m venv venv
. venv/bin/activate
python -m pip install -r requirements.txt


# git clone git@github.com:farfromrefug/rio-rgbify.git
cd rio-rgbify
python -m pip install -r requirements.txt
python -m pip install -e .
cd ..

# git clone git@github.com:farfromrefug/tippecanoe.git
cd tippecanoe
make -j
# make install
cd ..

# git clone --recurse-submodules -j8 git@github.com:farfromrefug/planetiler.git
cd planetiler
./scripts/build.sh
cd ..

# git clone --recurse-submodules git@github.com:kevinkreiser/prime_server.git
cd prime_server
./autogen.sh
./configure
make test -j$NPROC
cd ..

# git clone --recurse-submodules https://github.com/valhalla/valhalla.git
cd valhalla
# will build to ./build
cmake -B build -DCMAKE_BUILD_TYPE=Release
make -C build -j$NPROC
cd ..
