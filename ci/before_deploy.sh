#!/usr/bin/env bash
# This script takes care of building your crate and packaging it for release

set -ex

main() {
    local src=$(pwd) \
          stage=

    case $TRAVIS_OS_NAME in
        linux)
            stage=$(mktemp -d)
            ;;
        osx)
            stage=$(mktemp -d -t tmp)
            ;;
    esac

    test -f Cargo.lock || cargo generate-lockfile

    cross rustc --bin $CRATE_NAME --target $TARGET --release -- -C lto

    cp target/$TARGET/release/$CRATE_NAME $stage/ || cp target/$TARGET/release/$CRATE_NAME.exe $stage/

    cd $stage
    zip $src/$CRATE_NAME-$TRAVIS_TAG-$TARGET.zip *
    cd $src

    rm -rf $stage
}

main