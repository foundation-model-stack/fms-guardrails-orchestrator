#[derive(Default, Debug, Clone, PartialEq)]
pub struct Chunk {
    pub offset: usize,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

// TODO: conversion impls
